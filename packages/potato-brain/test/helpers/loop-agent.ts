// A bare Agent with one scripted model and one "probe" computer tool, for tests of the tool loop itself.
import { Agent } from "../../src/core/loop.ts";
import type { ExtraTool } from "../../src/core/loop.ts";
import { initialMood } from "../../src/persona/mood.ts";
import type { ChatRequest, ChatResult, Msg, Provider, ToolCall } from "../../src/types.ts";

export const usage = { promptTokens: 10, completionTokens: 5, cacheHitTokens: 0, cacheMissTokens: 10 };
const at = (): Date => new Date(2026, 10, 13, 10, 0, 0);

export const callOf = (id: string, name: string, args: unknown): ToolCall => ({
  id,
  name,
  arguments: typeof args === "string" ? args : JSON.stringify(args),
});
export const wantsCall = (...calls: ToolCall[]): ChatResult => ({ text: "", usage, model: "m", toolCalls: calls });
export const cutOffCall = (...calls: ToolCall[]): ChatResult => ({ text: "", usage, model: "m", toolCalls: calls, cutOff: true });
export const answerOf = (text = "done"): ChatResult => ({ text, usage, model: "m" });

/** Each step is a result or an error to throw (or a function of the request); every request seen is kept. */
export class ScriptedModel implements Provider {
  readonly name = "scripted";
  requests: ChatRequest[] = [];
  private steps: Array<ChatResult | Error | ((req: ChatRequest) => ChatResult | Error)>;
  private after: ((n: number, req: ChatRequest) => ChatResult) | undefined;
  constructor(
    steps: Array<ChatResult | Error | ((req: ChatRequest) => ChatResult | Error)>,
    after?: (n: number, req: ChatRequest) => ChatResult,
  ) {
    this.steps = steps;
    this.after = after;
  }
  async chat(req: ChatRequest): Promise<ChatResult> {
    this.requests.push(req);
    let step = this.steps.shift();
    if (step === undefined) step = this.after ? this.after(this.requests.length, req) : answerOf("end");
    if (typeof step === "function") step = step(req);
    if (step instanceof Error) throw step;
    return step;
  }
}

export interface LoopRig {
  agent: Agent;
  waits: number[];
  logs: string[];
  /** how many times each tool ran */
  runs: Record<string, number>;
}

/** `probe` is a computer-gated tool; its output is `output(n, args)`, where n counts its runs. */
export function loopRig(
  provider: Provider,
  over: { output?: (n: number, args: Record<string, unknown>) => string; extra?: ExtraTool[]; sleep?: (ms: number, s?: AbortSignal) => Promise<void> } = {},
): LoopRig {
  const waits: number[] = [];
  const logs: string[] = [];
  const runs: Record<string, number> = {};
  const probe: ExtraTool = {
    gate: "sandbox",
    spec: { name: "probe", description: "probe", parameters: { type: "object", properties: {} } },
    run: (args) => {
      runs["probe"] = (runs["probe"] ?? 0) + 1;
      return over.output ? over.output(runs["probe"], args) : `output ${runs["probe"]}`;
    },
  };
  const agent = new Agent({
    provider,
    persona: [{ id: "p", volatility: 0, content: "persona" }],
    mood: initialMood(at()),
    now: at,
    extraTools: [probe, ...(over.extra ?? [])],
    random: () => 0.5,
    log: (l) => logs.push(l),
    sleep: over.sleep ?? (async (ms) => void waits.push(ms)),
  });
  return { agent, waits, logs, runs };
}

export const toolMessages = (r: ChatRequest): Msg[] => r.messages.filter((m) => m.role === "tool");
