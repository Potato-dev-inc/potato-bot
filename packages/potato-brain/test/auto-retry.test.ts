import { test } from "node:test";
import assert from "node:assert/strict";
import { Agent } from "../src/core/loop.ts";
import type { ExtraTool } from "../src/core/loop.ts";
import { CodedError } from "../src/core/errors.ts";
import type { ErrorCode } from "../src/core/errors.ts";
import { OLD_OUTPUT_LINE, RETRY_TURN_CAP, shrinkOldToolResults } from "../src/core/retry.ts";
import { initialMood } from "../src/persona/mood.ts";
import type { ChatRequest, ChatResult, Msg, Provider } from "../src/types.ts";

const usage = { promptTokens: 10, completionTokens: 5, cacheHitTokens: 0, cacheMissTokens: 10 };
const at = () => new Date(2026, 9, 29, 10, 0, 0);
const down = (code: ErrorCode = "PROVIDER_DOWN") => new CodedError(code, "x", 503, "http 503");
const callRound = (id: string): ChatResult => ({
  text: "",
  usage,
  model: "m",
  toolCalls: [{ id, name: "probe", arguments: JSON.stringify({ id }) }],
});
const answer = (text = "done"): ChatResult => ({ text, usage, model: "m" });

/** Each step is a result or an error to throw; the requests it saw are kept. */
class Scripted implements Provider {
  readonly name = "scripted";
  requests: ChatRequest[] = [];
  private steps: Array<ChatResult | Error>;
  private onCall: ((n: number) => void) | undefined;
  constructor(steps: Array<ChatResult | Error>, onCall?: (n: number) => void) {
    this.steps = steps;
    this.onCall = onCall;
  }
  async chat(req: ChatRequest): Promise<ChatResult> {
    this.requests.push(req);
    this.onCall?.(this.requests.length);
    const step = this.steps.shift() ?? answer("end");
    if (step instanceof Error) throw step;
    return step;
  }
}

function setup(provider: Provider, over: { sleep?: (ms: number, s?: AbortSignal) => Promise<void>; sandbox?: boolean } = {}) {
  const waits: number[] = [];
  const logs: string[] = [];
  let runs = 0;
  const probe: ExtraTool = {
    ...(over.sandbox ? { gate: "sandbox" as const } : {}),
    spec: { name: "probe", description: "probe", parameters: { type: "object", properties: {} } },
    run: () => `output ${++runs} ${"x".repeat(50)}`,
  };
  const agent = new Agent({
    provider,
    persona: [{ id: "p", volatility: 0, content: "persona" }],
    mood: initialMood(at()),
    now: at,
    extraTools: [probe],
    random: () => 0.5,
    log: (l) => logs.push(l),
    sleep: over.sleep ?? (async (ms) => void waits.push(ms)),
  });
  return { agent, waits, logs, runs: () => runs };
}

const toolMsgs = (r: ChatRequest): Msg[] => r.messages.filter((m) => m.role === "tool");

test("an outage then success keeps earlier tool results and does not re-run tools", async () => {
  const p = new Scripted([callRound("c1"), callRound("c2"), down(), down(), answer("all good")]);
  const t = setup(p);
  const notes: string[] = [];
  const res = await t.agent.turn("go", { onNote: (x) => (notes.push(x), true) });
  assert.equal(res.segments[0], "all good");
  assert.equal(t.runs(), 2, "tools ran once each");
  assert.deepEqual(t.waits, [15_000, 45_000]);
  const last = p.requests.at(-1)!;
  assert.equal(toolMsgs(last).length, 2);
  assert.equal(p.requests.length, 5);
  assert.equal(notes.length, 1);
});

test("the shrink starts at the second retry, keeps the last 6 results and every call/result pair, and stays", async () => {
  const steps: Array<ChatResult | Error> = [];
  for (let i = 1; i <= 9; i++) steps.push(callRound(`c${i}`));
  steps.push(down(), down(), answer("ok"));
  const p = new Scripted(steps);
  // 9 tool rounds need the bigger round budget of a computer turn
  const t = setup(p, { sandbox: true });
  const res = await t.agent.turn("go", { sandbox: true });
  assert.equal(res.segments[0], "ok");
  const [, first, second] = p.requests.slice(-3) as [ChatRequest, ChatRequest, ChatRequest];
  assert.equal(toolMsgs(first).filter((m) => m.content === OLD_OUTPUT_LINE).length, 0, "first retry is not shrunk");
  const tm = toolMsgs(second);
  assert.equal(tm.length, 9);
  assert.equal(tm.filter((m) => m.content === OLD_OUTPUT_LINE).length, 3);
  assert.ok(tm.slice(-6).every((m) => m.content !== OLD_OUTPUT_LINE));
  const ids = second.messages.flatMap((m) => (m.toolCalls ?? []).map((c) => c.id)).sort();
  assert.deepEqual(tm.map((m) => m.toolCallId).sort(), ids);
  assert.equal(second.messages[0]!.content, p.requests[0]!.messages[0]!.content, "system text untouched");
  assert.equal(
    second.messages.filter((m) => m.role === "user").length,
    p.requests[0]!.messages.filter((m) => m.role === "user").length,
  );
});

test("shrinkOldToolResults drops pictures of old results and returns the same array when nothing to do", () => {
  const msgs: Msg[] = [
    { role: "user", content: "u" },
    { role: "assistant", content: "", toolCalls: [{ id: "a", name: "probe", arguments: "{}" }] },
    { role: "tool", toolCallId: "a", content: "big", images: [{ mimeType: "image/png", data: "AA" }] },
  ];
  assert.equal(shrinkOldToolResults(msgs), msgs);
  const out = shrinkOldToolResults(msgs, 0);
  assert.equal(out[2]!.content, OLD_OUTPUT_LINE);
  assert.equal(out[2]!.images, undefined);
  assert.equal(out[2]!.toolCallId, "a");
});

test("Stop during a wait ends the turn at once with no further model call", async () => {
  const ctl = new AbortController();
  const p = new Scripted([callRound("c1"), down(), answer("never")]);
  const t = setup(p, {
    sleep: (_ms, signal) =>
      new Promise((_, reject) => {
        queueMicrotask(() => ctl.abort());
        signal?.addEventListener("abort", () => reject(new CodedError("NETWORK", "stopped", undefined, "stopped")), {
          once: true,
        });
      }),
  });
  await assert.rejects(
    t.agent.turn("go", { signal: ctl.signal }),
    (e: unknown) => e instanceof CodedError && e.reason === "stopped",
  );
  assert.equal(p.requests.length, 2, "no model call after Stop");
  assert.ok(p.requests.every((r) => r.signal === ctl.signal), "the signal reaches the provider");
});

test("a sleep that ignores the signal still cannot lead to another call after Stop", async () => {
  const ctl = new AbortController();
  const p = new Scripted([down(), answer("never")]);
  const t = setup(p, { sleep: async () => ctl.abort() });
  await assert.rejects(t.agent.turn("go", { signal: ctl.signal }));
  assert.equal(p.requests.length, 1);
});

test("a hold or the daily limit during a wait stops the retries", async () => {
  let halted = false;
  const p = new Scripted([callRound("c1"), down(), answer("never")]);
  const t = setup(p, { sleep: async () => void (halted = true) });
  await assert.rejects(t.agent.turn("go", { halted: () => halted }));
  assert.equal(p.requests.length, 2);
  const p2 = new Scripted([down(), answer("never")]);
  const t2 = setup(p2);
  await assert.rejects(t2.agent.turn("go", { halted: () => p2.requests.length > 0 }));
  assert.equal(p2.requests.length, 1);
  assert.deepEqual(t2.waits, []);
});

test("auth and limit errors are not retried", async () => {
  for (const code of ["PROVIDER_AUTH", "PROVIDER_LIMIT", "INTERNAL"] as const) {
    const p = new Scripted([down(code), answer("never")]);
    const t = setup(p);
    await assert.rejects(t.agent.turn("go"), (e: unknown) => e instanceof CodedError && e.code === code);
    assert.equal(p.requests.length, 1);
    assert.deepEqual(t.waits, []);
  }
});

test("a NETWORK error is retried too", async () => {
  const p = new Scripted([down("NETWORK"), answer("back")]);
  const t = setup(p);
  const res = await t.agent.turn("go");
  assert.equal(res.segments[0], "back");
  assert.deepEqual(t.waits, [15_000]);
});

test("at most 4 retries in a round, then the turn fails as it does today", async () => {
  const p = new Scripted([down(), down(), down(), down(), down(), answer("never")]);
  const t = setup(p);
  await assert.rejects(t.agent.turn("go"), (e: unknown) => e instanceof CodedError && e.code === "PROVIDER_DOWN");
  assert.equal(p.requests.length, 5);
  assert.deepEqual(t.waits, [15_000, 45_000, 120_000, 240_000]);
});

test("at most 8 retried calls in a turn across rounds", async () => {
  const steps: Array<ChatResult | Error> = [];
  for (let r = 1; r <= 2; r++) steps.push(down(), down(), down(), callRound(`c${r}`));
  steps.push(down(), down(), down(), down(), answer("never"));
  const p = new Scripted(steps);
  const t = setup(p);
  await assert.rejects(t.agent.turn("go"));
  assert.equal(t.waits.length, RETRY_TURN_CAP);
  assert.equal(p.requests.length, 2 * 4 + 3);
});

test("only one note per turn, and a quiet pure-chat turn says nothing under 30 s", async () => {
  const p = new Scripted([callRound("c1"), down(), callRound("c2"), down(), answer("ok")]);
  const t = setup(p);
  const notes: string[] = [];
  await t.agent.turn("go", { onNote: (x) => (notes.push(x), true) });
  assert.equal(notes.length, 1);
  assert.match(notes[0]!, /stopped answering\. Retrying by itself\. Step 1 so far\./);

  const q = new Scripted([down(), answer("hi")]);
  const tq = setup(q);
  const quiet: string[] = [];
  await tq.agent.turn("hello", { onNote: (x) => (quiet.push(x), true) });
  assert.equal(quiet.length, 0, "a 15 s wait on pure chat is quiet");

  const q2 = new Scripted([down(), down(), answer("hi")]);
  const tq2 = setup(q2);
  const loud: string[] = [];
  await tq2.agent.turn("hello", { onNote: (x) => (loud.push(x), true) });
  assert.equal(loud.length, 1, "past 30 s of waiting it speaks, once");
  assert.doesNotMatch(loud[0]!, /Step/);
});

test("the final no-tools call is retried; exhausted retries give the stopped-after-steps line", async () => {
  const steps: Array<ChatResult | Error> = [callRound("c1"), callRound("c2"), callRound("c3"), callRound("c4")];
  steps.push(down(), answer("final text"));
  const p = new Scripted(steps);
  const t = setup(p);
  const res = await t.agent.turn("go");
  assert.equal(res.segments.join(""), "final text");
  assert.deepEqual(t.waits, [15_000]);

  const dead: Array<ChatResult | Error> = [callRound("c1"), callRound("c2"), callRound("c3"), callRound("c4")];
  for (let i = 0; i < 5; i++) dead.push(down());
  const p2 = new Scripted(dead);
  const t2 = setup(p2);
  const res2 = await t2.agent.turn("go");
  assert.match(res2.segments.join(""), /I stopped after 4 steps\. Say continue/);
  assert.equal(t2.waits.length, 4);
});

test("Stop on the final call ends the turn instead of giving the stopped-after-steps line", async () => {
  const ctl = new AbortController();
  const steps: Array<ChatResult | Error> = [callRound("c1"), callRound("c2"), callRound("c3"), callRound("c4"), down()];
  const p = new Scripted(steps, (n) => {
    if (n === 5) ctl.abort();
  });
  const t = setup(p);
  await assert.rejects(t.agent.turn("go", { signal: ctl.signal }));
});

test("one safe log line per retry: numbers and fixed words, never body text", async () => {
  const p = new Scripted([callRound("c1"), down(), answer("ok")]);
  const t = setup(p);
  await t.agent.turn("go");
  assert.equal(t.logs.length, 1);
  assert.match(
    t.logs[0]!,
    /^\[retry\] attempt=1\/4 wait=15s code=PROVIDER_DOWN reason="http 503" round=2 messages=\d+ approx_prompt_tokens=\d+/,
  );
  assert.doesNotMatch(t.logs[0]!, /output|persona/);
});
