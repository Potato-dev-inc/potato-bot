// Inside one turn the old terminal/file output stays bounded: the newest few results stay whole, older big ones become a
// head + tail stub, in steps (so the start of the prompt stays put), and a context-too-long error trims harder, once.
import { test } from "node:test";
import assert from "node:assert/strict";
import { CodedError, CONTEXT_TOO_LONG_REASON } from "../src/core/errors.ts";
import { DEFAULT_TRIM, trimTurnToolResults } from "../src/core/turn-budget.ts";
import type { Msg } from "../src/types.ts";
import { answerOf, callOf, loopRig, ScriptedModel, toolMessages, wantsCall } from "./helpers/loop-agent.ts";

const big = (n: number, size = 20_000): string => `HEAD-${n} ` + "x".repeat(size - 30) + ` TAIL-${n}`;
const toolChars = (msgs: Msg[]): number => msgs.filter((m) => m.role === "tool").reduce((s, m) => s + m.content.length, 0);

function roundOf(n: number, size = 20_000): Msg[] {
  return [
    { role: "assistant", content: "", toolCalls: [{ id: `c${n}`, name: "probe", arguments: "{}" }] },
    { role: "tool", toolCallId: `c${n}`, content: big(n, size) },
  ];
}

test("60 rounds of 20k-character results stay under the budget, newest kept whole, every call still paired", () => {
  let msgs: Msg[] = [{ role: "system", content: "sys" }, { role: "user", content: "go" }];
  let peak = 0;
  const trimmedAt: number[] = [];
  for (let n = 1; n <= 60; n++) {
    msgs = [...msgs, ...roundOf(n)];
    const next = trimTurnToolResults(msgs);
    if (next !== msgs) trimmedAt.push(n);
    msgs = next;
    peak = Math.max(peak, toolChars(msgs));
  }
  assert.ok(peak <= DEFAULT_TRIM.budget, `peak ${peak} is over ${DEFAULT_TRIM.budget}`);
  assert.ok(toolChars(msgs) <= DEFAULT_TRIM.budget);
  const tools = msgs.filter((m) => m.role === "tool");
  assert.equal(tools.length, 60, "no result is dropped");
  for (const m of tools.slice(-DEFAULT_TRIM.keepFull)) assert.ok(m.content.length > 19_900, "the newest stay whole");
  const first = tools[0]!.content;
  assert.ok(first.length < 1_500, "the oldest is small now");
  const calls = msgs.flatMap((m) => (m.toolCalls ?? []).map((c) => c.id));
  assert.deepEqual(tools.map((m) => m.toolCallId), calls);
  assert.ok(trimmedAt.length > 1 && trimmedAt.length < 30, `trimmed in steps, not every round (${trimmedAt.length} times)`);
});

test("a stub keeps the start and the end of the output and says it was trimmed", () => {
  let msgs: Msg[] = [];
  for (let n = 1; n <= 12; n++) msgs = [...msgs, ...roundOf(n)];
  const out = trimTurnToolResults(msgs);
  const stub = out.find((m) => m.role === "tool")!.content;
  assert.match(stub, /^HEAD-1 /);
  assert.match(stub, /TAIL-1$/);
  assert.match(stub, /\[trimmed: \d+ characters/);
});

test("nothing changes under the budget, and the same array comes back", () => {
  const msgs: Msg[] = [...roundOf(1), ...roundOf(2)];
  assert.equal(trimTurnToolResults(msgs), msgs);
});

test("small results and the pictures-with-text of the newest are left alone; a second pass changes nothing more", () => {
  let msgs: Msg[] = [];
  for (let n = 1; n <= 12; n++) msgs = [...msgs, ...roundOf(n, n % 2 === 0 ? 500 : 20_000)];
  const once = trimTurnToolResults(msgs);
  assert.equal(once.filter((m) => m.role === "tool" && m.content.length < 600).length, 6, "small ones are whole");
  assert.equal(trimTurnToolResults(once), once, "already under the line: no change");
});

test("in a real turn, 60 rounds never send more than the budget, and an earlier stub never changes", async () => {
  const p = new ScriptedModel([], (n) => (n <= 60 ? wantsCall(callOf(`r${n}`, "probe", { n })) : answerOf("all done")));
  const t = loopRig(p, { output: (n) => big(n) });
  const res = await t.agent.turn("render it", { sandbox: true });
  assert.equal(res.segments[0], "all done");
  assert.equal(t.runs["probe"], 60);
  let peak = 0;
  for (const r of p.requests) peak = Math.max(peak, toolChars(r.messages));
  assert.ok(peak <= DEFAULT_TRIM.budget, `peak ${peak}`);
  // the prompt prefix stays stable: whatever was in the request before is the same text in the next one, unless it was
  // trimmed in between (a stub only ever gets smaller, and a message that is not touched is identical)
  const rewrites = new Map<number, number>();
  for (let k = 1; k < p.requests.length; k++) {
    const a = toolMessages(p.requests[k - 1]!);
    const b = toolMessages(p.requests[k]!);
    for (let i = 0; i < a.length; i++) if (a[i]!.content !== b[i]!.content) rewrites.set(i, (rewrites.get(i) ?? 0) + 1);
  }
  assert.ok([...rewrites.values()].every((n) => n <= 2), "each result is rewritten at most twice: whole, stub, one line");
});

test("a context-too-long error trims the old outputs harder and asks again once", async () => {
  const limit = 70_000;
  const tooLong = new CodedError("INTERNAL", "x 400", 400, CONTEXT_TOO_LONG_REASON);
  const seen: number[] = [];
  const failed: number[] = [];
  const inner = new ScriptedModel([], (n) => (n <= 5 ? wantsCall(callOf(`r${n}`, "probe", { n })) : answerOf("finished")));
  const provider = {
    name: "limited",
    chat: async (req: Parameters<typeof inner.chat>[0]) => {
      const size = toolChars(req.messages);
      seen.push(size);
      if (size > limit) {
        failed.push(seen.length);
        throw tooLong;
      }
      return inner.chat(req);
    },
  };
  // 4 results of 18k are 72k: the fifth request is the first over the limit (the default trim only starts at 160k)
  const t = loopRig(provider, { output: (n) => big(n, 18_000) });
  const res = await t.agent.turn("go", { sandbox: true });
  assert.equal(res.segments[0], "finished");
  assert.equal(failed.length, 1, "refused once");
  assert.ok(seen[failed[0]!]! <= limit, "the very next request was trimmed and fit");
  assert.equal(t.waits.length, 0, "no wait: it is not an outage");
});

test("a second context-too-long error in the same turn is not retried again", async () => {
  const tooLong = new CodedError("INTERNAL", "x 400", 400, CONTEXT_TOO_LONG_REASON);
  let asks = 0;
  const provider = {
    name: "always-long",
    chat: async (): Promise<never> => {
      asks++;
      throw tooLong;
    },
  };
  const t = loopRig(provider);
  await assert.rejects(t.agent.turn("go", { sandbox: true }), (e: unknown) => e instanceof CodedError && e.reason === CONTEXT_TOO_LONG_REASON);
  assert.equal(asks, 1, "nothing to trim, so no retry at all");
});

test("with old output to trim, the retry happens once and a second refusal ends the turn", async () => {
  const tooLong = new CodedError("INTERNAL", "x 400", 400, CONTEXT_TOO_LONG_REASON);
  let asks = 0;
  const inner = new ScriptedModel([wantsCall(callOf("a", "probe", {})), wantsCall(callOf("b", "probe", {})), wantsCall(callOf("c", "probe", {}))]);
  const provider = {
    name: "long-late",
    chat: async (req: Parameters<typeof inner.chat>[0]) => {
      asks++;
      if (asks >= 4) throw tooLong;
      return inner.chat(req);
    },
  };
  const t = loopRig(provider, { output: (n) => big(n, 18_000) });
  await assert.rejects(t.agent.turn("go", { sandbox: true }), (e: unknown) => e instanceof CodedError && e.reason === CONTEXT_TOO_LONG_REASON);
  assert.equal(asks, 5, "3 good rounds, the refused one, one retry");
});
