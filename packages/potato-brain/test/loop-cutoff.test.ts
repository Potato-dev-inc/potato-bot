// A reply the provider cut off at its output limit: its tool calls never run, the model is told to write smaller
// pieces, and the turn goes on. Broken JSON arguments are an error the model hears, never a silent {}.
import { test } from "node:test";
import assert from "node:assert/strict";
import { answerOf, callOf, cutOffCall, loopRig, ScriptedModel, toolMessages, wantsCall } from "./helpers/loop-agent.ts";

test("a cut-off call is not run; the model is told, and the turn continues with its next try", async () => {
  const p = new ScriptedModel([
    cutOffCall(callOf("c1", "probe", '{"command":"echo hel')),
    wantsCall(callOf("c2", "probe", { command: "echo hi" })),
    answerOf("finished"),
  ]);
  const t = loopRig(p);
  const res = await t.agent.turn("go", { sandbox: true });
  assert.equal(res.segments[0], "finished");
  assert.equal(t.runs["probe"], 1, "only the whole call ran");
  const second = p.requests[1]!;
  const tm = toolMessages(second);
  assert.equal(tm.length, 1);
  assert.equal(tm[0]!.toolCallId, "c1", "the cut-off call is still answered, so ids stay paired");
  assert.match(tm[0]!.content, /cut off before it finished/);
  assert.match(tm[0]!.content, /smaller pieces/);
  const asked = second.messages.flatMap((m) => m.toolCalls ?? []);
  assert.equal(asked[0]!.arguments, "{}", "the broken arguments are not sent back");
});

test("replies cut off three times in a row end the turn with a plain sentence", async () => {
  const p = new ScriptedModel([], (n) => cutOffCall(callOf(`c${n}`, "probe", '{"x":')));
  const t = loopRig(p);
  const res = await t.agent.turn("go", { sandbox: true });
  assert.match(res.segments.join(" "), /cut off in the middle of a step/);
  assert.equal(t.runs["probe"] ?? 0, 0);
  assert.equal(p.requests.length, 3);
});

test("a good round in between resets the cut-off count", async () => {
  const p = new ScriptedModel([
    cutOffCall(callOf("a", "probe", "{")),
    cutOffCall(callOf("b", "probe", "{")),
    wantsCall(callOf("c", "probe", {})),
    cutOffCall(callOf("d", "probe", "{")),
    cutOffCall(callOf("e", "probe", "{")),
    answerOf("ok"),
  ]);
  const t = loopRig(p);
  const res = await t.agent.turn("go", { sandbox: true });
  assert.equal(res.segments[0], "ok");
  assert.equal(t.runs["probe"], 1);
});

test("arguments that are not valid JSON are an error to the model, not an empty call", async () => {
  const p = new ScriptedModel([wantsCall(callOf("c1", "probe", '{"command": "ls')), answerOf("ok")]);
  const t = loopRig(p);
  await t.agent.turn("go", { sandbox: true });
  assert.equal(t.runs["probe"] ?? 0, 0, "the tool did not run with {}");
  const tm = toolMessages(p.requests[1]!);
  assert.match(tm[0]!.content, /not a valid JSON object/);
  assert.equal(tm[0]!.toolCallId, "c1");
});

test("empty arguments are still fine for a tool with no arguments", async () => {
  const p = new ScriptedModel([wantsCall(callOf("c1", "probe", "")), answerOf("ok")]);
  const t = loopRig(p);
  await t.agent.turn("go", { sandbox: true });
  assert.equal(t.runs["probe"], 1);
});
