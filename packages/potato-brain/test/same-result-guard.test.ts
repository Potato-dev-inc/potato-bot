// The loop guard also watches the result: the same call returning the same text again and again gets a note to the model
// on the 3rd time and ends the turn with a plain message on the 5th.
import { test } from "node:test";
import assert from "node:assert/strict";
import { isLoopGuardStopText, noteToolResult, advanceToolCallLoopGuard, type ToolCallStreak } from "../src/lifted/tool-loop.ts";
import { answerOf, callOf, loopRig, ScriptedModel, toolMessages, wantsCall } from "./helpers/loop-agent.ts";

test("the same call with the same result: a note on the 3rd, the turn ends on the 5th", async () => {
  const p = new ScriptedModel([], (n) => wantsCall(callOf(`s${n}`, "probe", { path: "/render.log" })));
  const t = loopRig(p, { output: () => "frame 100/900 (still going)" });
  const res = await t.agent.turn("watch the render", { sandbox: true });
  const text = res.segments.join(" ");
  assert.match(text, /I ran probe 5 times in a row and got exactly the same result each time, so I stopped/);
  assert.equal(isLoopGuardStopText(text), true);
  assert.equal(t.runs["probe"], 5, "the fifth result is the one that ends the turn: no sixth call");
  // the note is in what the model saw after the 3rd result, not before
  const tools = toolMessages(p.requests.at(-1)!);
  assert.equal(tools.length, 4, "four results had gone back to the model");
  assert.equal(tools.filter((m) => m.content.includes("exactly the same result")).length, 1);
  assert.ok(!tools[0]!.content.includes("exactly the same result"));
  assert.match(tools[2]!.content, /same result 3 times in a row/);
});

test("the same call with a changing result is polling, not a loop", async () => {
  const p = new ScriptedModel([], (n) => (n <= 5 ? wantsCall(callOf(`s${n}`, "probe", { path: "/render.log" })) : answerOf("render is done")));
  const t = loopRig(p, { output: (n) => `frame ${n * 100}/900` });
  const res = await t.agent.turn("watch the render", { sandbox: true });
  assert.equal(res.segments[0], "render is done");
  assert.equal(t.runs["probe"], 5);
});

test("different calls with the same result do not count", async () => {
  const p = new ScriptedModel([], (n) => (n <= 8 ? wantsCall(callOf(`s${n}`, "probe", { path: `/file-${n}` })) : answerOf("done")));
  const t = loopRig(p, { output: () => "No such file" });
  const res = await t.agent.turn("look", { sandbox: true });
  assert.equal(res.segments[0], "done");
  assert.equal(t.runs["probe"], 8);
});

test("a different result in between restarts the count", () => {
  let s: ToolCallStreak = { key: undefined, count: 0 };
  const results = ["a", "a", "b", "b", "b", "b"];
  const flags: boolean[] = [];
  for (const r of results) {
    s = advanceToolCallLoopGuard(s, "t", { x: 1 }).streak;
    const step = noteToolResult(s, r);
    s = step.streak;
    flags.push(step.nudge);
  }
  assert.deepEqual(flags, [false, false, false, false, true, false], "the 3rd 'b' (5th call) gets the note");
  assert.equal(s.sameResultRun, 4);
});

test("the old rule still ends a run of six identical calls even when each result is new", async () => {
  const p = new ScriptedModel([], (n) => wantsCall(callOf(`s${n}`, "probe", {})));
  const t = loopRig(p, { output: (n) => `result ${n}` });
  const res = await t.agent.turn("go", { sandbox: true });
  assert.match(res.segments.join(" "), /I got stuck calling probe with the same input 6 times in a row/);
});
