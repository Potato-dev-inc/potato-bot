// Pictures a person attaches: shown to a model that can see, named as unseen to one that cannot, never kept in the history.
process.env.FISHD_PROVIDER = "stub";

import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { makeAgentFactory } from "../src/agents.ts";
import type { Config } from "../src/config.ts";
import { PotatoChannel, parseImages } from "../src/channels/potato.ts";
import { toWireAll } from "../src/core/openai.ts";
import type { ChatRequest, ChatResult, Provider } from "../src/types.ts";
import { startMockBridge } from "./helpers/mock-bridge.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const usage = { promptTokens: 10, completionTokens: 5, cacheHitTokens: 0, cacheMissTokens: 10 };

/** Each chat call waits for its gate (if any), then answers from the script. Counts how many run at once. */
class Gated implements Provider {
  readonly name = "gated";
  modelName: string;
  calls: ChatRequest[] = [];
  running = 0;
  maxRunning = 0;
  gates: Array<Promise<void> | undefined> = [];
  private script: Array<(req: ChatRequest) => ChatResult>;
  constructor(modelName: string, script: Array<(req: ChatRequest) => ChatResult>) {
    this.modelName = modelName;
    this.script = script;
  }
  async chat(req: ChatRequest): Promise<ChatResult> {
    const n = this.calls.length;
    this.calls.push(structuredClone({ ...req, signal: undefined }));
    this.running++;
    this.maxRunning = Math.max(this.maxRunning, this.running);
    try {
      await this.gates[n];
      const step = this.script[n];
      return step ? step(req) : { text: "ok", usage, model: "g" };
    } finally {
      this.running--;
    }
  }
}

const say = (text: string): ChatResult => ({ text, usage, model: "g" });

async function rig(provider: Provider) {
  const root = mkdtempSync(join(tmpdir(), "potato-steer-"));
  const bridge = await startMockBridge();
  const config: Config = {
    model: "stub",
    temperature: 1,
    maxTokens: 600,
    vaultRoot: join(root, "vault"),
    personaDir: join(HERE, "..", "persona"),
    shareDir: join(root, "share"),
    staticBudget: 4000,
    stableBudget: 900,
    stateDir: join(root, "state"),
    hotTurns: 24,
    voice: { enabled: false, probability: 0, maxChars: 60 },
  };
  const factory = makeAgentFactory({ config, pack: () => "fish", share: false, sleep: async () => {}, provider: () => provider });
  const channel = new PotatoChannel({
    url: bridge.url,
    token: "tok-test",
    info: { version: "t", provider: "stub", packs: ["fish", "potato", "pro"], surfaces: ["app"] },
    agentFor: (c, tools) => factory(c, tools),
    setPack: () => {},
    onExit: () => {},
    log: () => {},
    waitMs: 120,
    retryMs: 10,
    retryMaxMs: 30,
    maxOutageMs: 400,
  });
  void channel.start();
  return {
    bridge,
    async done() {
      await channel.stop();
      await bridge.close();
      rmSync(root, { recursive: true, force: true });
    },
  };
}

const send = (id: string, text: string, over: Record<string, unknown> = {}): Record<string, unknown> => ({
  type: "send",
  id,
  conversationId: "app:dm:bot1",
  surface: "app",
  from: { id: "owner", name: "Pat", trust: "owner_pc" },
  text,
  tools: "full",
  ...over,
});


const PNG = "iVBORw0KGgo=";
const pic = { mimeType: "image/png", data: PNG };
const LINE = "[Attached image: shot.png (50 KB), saved on your computer at /home/pm/attachments/1-1-shot.png]";

test("a picture the person attaches reaches a model that can see, on their own line, and not the history", async () => {
  const p = new Gated("gpt-5", [() => say("nice picture"), () => say("still here")]);
  const r = await rig(p);
  try {
    r.bridge.push(send("m1", `look

${LINE}`, { images: [pic] }));
    await r.bridge.waitFor((e) => e["type"] === "typing" && e["on"] === false);
    const users = (p.calls[0] as ChatRequest).messages.filter((m) => m.role === "user");
    const last = users[users.length - 1];
    assert.deepEqual(last?.images, [pic]);
    assert.ok(String(last?.content).includes(LINE));
    assert.ok(!String(last?.content).includes("can't view images"));
    // the next turn: the old picture is not sent again (the file line stays in the text)
    r.bridge.push(send("m2", "and now?"));
    await r.bridge.waitFor((e) => e["type"] === "reply" && e["inReplyTo"] === "m2");
    const second = (p.calls[1] as ChatRequest).messages;
    assert.ok(second.every((m) => !m.images), "no picture in the second turn");
    assert.ok(second.some((m) => String(m.content).includes(LINE)));
  } finally {
    await r.done();
  }
});

test("a model that cannot see gets the text only, and the line says so", async () => {
  const p = new Gated("deepseek-flash", [() => say("I cannot look, but I know the file")]);
  const r = await rig(p);
  try {
    r.bridge.push(send("m1", `look

${LINE}`, { images: [pic] }));
    await r.bridge.waitFor((e) => e["type"] === "typing" && e["on"] === false);
    const msgs = (p.calls[0] as ChatRequest).messages;
    assert.ok(msgs.every((m) => !m.images));
    const last = msgs.filter((m) => m.role === "user").pop();
    assert.ok(String(last?.content).includes(LINE));
    assert.ok(String(last?.content).includes("can't view images"));
  } finally {
    await r.done();
  }
});

test("a stranger's turn never gets a picture", async () => {
  const p = new Gated("gpt-5", [() => say("ok")]);
  const r = await rig(p);
  try {
    r.bridge.push(send("m1", "hi", { images: [pic], from: { id: "x", name: "Sam", trust: "others" }, tools: "no_memory" }));
    await r.bridge.waitFor((e) => e["type"] === "typing" && e["on"] === false);
    assert.ok((p.calls[0] as ChatRequest).messages.every((m) => !m.images));
  } finally {
    await r.done();
  }
});

test("only good pictures are taken: known type, plain base64, at most four", () => {
  const good = { mimeType: "image/jpeg", data: "/9j/4AAQ" };
  assert.deepEqual(parseImages([good, { mimeType: "image/svg+xml", data: PNG }, { mimeType: "image/png", data: "not base64!" }, { mimeType: "image/png" }, "x", null]), [good]);
  assert.deepEqual(parseImages([good, good, good, good, good, good]).length, 4);
  assert.deepEqual(parseImages(undefined), []);
  assert.deepEqual(parseImages({}), []);
});

test("the wire format carries a user's pictures as image parts, only when the model sees", () => {
  const msgs = [{ role: "user" as const, content: "look", images: [pic] }];
  const seen = toWireAll(msgs) as Array<{ content: unknown }>;
  assert.deepEqual(seen[0]?.content, [
    { type: "text", text: "look" },
    { type: "image_url", image_url: { url: `data:image/png;base64,${PNG}` } },
  ]);
  const blind = toWireAll(msgs, false) as Array<{ content: unknown }>;
  assert.equal(blind[0]?.content, "look");
});
