// The bot's computer through the channel: a fake app (the mock bridge plus a little answering machine) and a scripted
// model. Shared by test/sandbox-channel.test.ts and the long-job eval.
process.env.FISHD_PROVIDER = "stub";
delete process.env.FISHD_VISION;

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { makeAgentFactory } from "../../src/agents.ts";
import type { Config } from "../../src/config.ts";
import { PotatoChannel } from "../../src/channels/potato.ts";
import type { PotatoChannelOptions } from "../../src/channels/potato.ts";
import type { ChatRequest, ChatResult, Provider, ToolCall, Usage } from "../../src/types.ts";
import { startMockBridge } from "./mock-bridge.ts";
import type { MockBridge } from "./mock-bridge.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
export const PERSONA = join(HERE, "..", "..", "persona");

export const usage = (): Usage => ({ promptTokens: 10, completionTokens: 5, cacheHitTokens: 0, cacheMissTokens: 10 });

/** A model that answers from a list, and from a function once the list is empty. */
export class Scripted implements Provider {
  readonly name = "scripted";
  calls: ChatRequest[] = [];
  private scripts: ChatResult[];
  private after: ((n: number) => ChatResult) | undefined;
  constructor(scripts: ChatResult[], after?: (n: number) => ChatResult) {
    this.scripts = scripts;
    this.after = after;
  }
  async chat(req: ChatRequest): Promise<ChatResult> {
    this.calls.push(structuredClone({ ...req, signal: undefined }));
    const next = this.scripts.shift();
    if (next) return next;
    return this.after ? this.after(this.calls.length) : { text: "done", usage: usage(), model: "scripted" };
  }
}

export const call = (name: string, args: Record<string, unknown>, id = `c-${Math.random().toString(36).slice(2, 8)}`): ToolCall => ({
  id,
  name,
  arguments: JSON.stringify(args),
});

export const wants = (calls: ToolCall[], model = "scripted"): ChatResult => ({ text: "", usage: usage(), model, toolCalls: calls });
export const says = (text: string, model = "scripted"): ChatResult => ({ text, usage: usage(), model });

export interface Rig {
  bridge: MockBridge;
  channel: PotatoChannel;
  provider: Scripted;
  /** every sandbox_call event the brain posted */
  sent(): Array<Record<string, unknown>>;
  /** the app answers each sandbox_call with this (default: nothing, the test pushes results by hand) */
  answer(fn: (event: Record<string, unknown>) => Record<string, unknown> | null): void;
  done(): Promise<void>;
}

export async function rig(provider: Scripted, channel: Partial<PotatoChannelOptions> = {}): Promise<Rig> {
  const root = mkdtempSync(join(tmpdir(), "potato-sbx-"));
  const bridge = await startMockBridge();
  const config: Config = {
    model: "stub",
    temperature: 1,
    maxTokens: 600,
    vaultRoot: join(root, "vault"),
    personaDir: PERSONA,
    shareDir: join(root, "share"),
    staticBudget: 4000,
    stableBudget: 900,
    stateDir: join(root, "state"),
    hotTurns: 24,
    voice: { enabled: false, probability: 0, maxChars: 60 },
  };
  const factory = makeAgentFactory({ config, pack: () => "fish", share: false, sleep: async () => {}, provider: () => provider });
  const ch = new PotatoChannel({
    url: bridge.url,
    token: "tok-test",
    info: { version: "0.0.0-test", provider: "stub", packs: ["fish", "potato"], surfaces: ["app"] },
    agentFor: (conversationId, tools) => factory(conversationId, tools),
    waitMs: 60,
    retryMs: 10,
    retryMaxMs: 30,
    maxOutageMs: 2000,
    ...channel,
  });
  void ch.start();

  let answering: ((event: Record<string, unknown>) => Record<string, unknown> | null) | null = null;
  const answered = new Set<string>();
  const timer = setInterval(() => {
    if (!answering) return;
    for (const event of bridge.events) {
      const id = String(event["callId"] ?? "");
      if (event["type"] !== "sandbox_call" || answered.has(id)) continue;
      answered.add(id);
      const item = answering(event);
      if (item) bridge.push({ type: "sandbox_result", callId: id, ...item });
    }
  }, 5);

  return {
    bridge,
    channel: ch,
    provider,
    sent: () => bridge.events.filter((e) => e["type"] === "sandbox_call"),
    answer: (fn) => {
      answering = fn;
    },
    async done() {
      clearInterval(timer);
      await ch.stop();
      await bridge.close();
      rmSync(root, { recursive: true, force: true });
    },
  };
}

export const sendMsg = (over: Record<string, unknown> = {}): Record<string, unknown> => ({
  type: "send",
  id: "m1",
  conversationId: "app:dm:bot1",
  surface: "app",
  from: { id: "owner", name: "Pat", trust: "owner_pc" },
  text: "hello",
  tools: "full",
  ...over,
});

export const withComputer = (over: Record<string, unknown> = {}): Record<string, unknown> => sendMsg({ capabilities: { sandbox: true }, ...over });

export const okResult = (result: Record<string, unknown>): Record<string, unknown> => ({ ok: true, result });
export const failed = (code: string, message: string, needsYou = false): Record<string, unknown> => ({
  ok: false,
  error: { code, message, ...(needsYou ? { needsYou: true } : {}) },
});

