import { test } from "node:test";
import assert from "node:assert/strict";
import { askUserTool, requestApprovalTool, CARD_WAIT_TEXT, REQUEST_APPROVAL_NAME } from "../src/core/question-tools.ts";
import { JOB_TURN_NOTE } from "../src/core/job-tools.ts";
import { ROOM_OFF_TOOLS, jobTurnStepsText, stoppedAfterStepsText } from "../src/core/loop.ts";
import type { SandboxReply } from "../src/core/sandbox-tools.ts";

const ctx = { turn: 1, where: undefined, model: "m" };
type Out = string | { text: string; endTurn?: boolean; isError?: boolean };
const run = async (tool: { run: (a: Record<string, unknown>, c: typeof ctx) => Out | Promise<Out> }, args: Record<string, unknown>) => {
  const out = await tool.run(args, ctx);
  return typeof out === "string" ? { text: out } : out;
};

test("request_approval: owner-person only, off in a room, sends action and why to the app", async () => {
  const sent: Array<[string, Record<string, unknown>]> = [];
  const tool = requestApprovalTool({
    call: async (name, args) => {
      sent.push([name, args]);
      return { ok: true, result: { card: true, cardId: "card-1" } };
    },
  });
  assert.equal(tool.gate, "owner_person");
  assert.equal(tool.spec.name, REQUEST_APPROVAL_NAME);
  assert.ok(ROOM_OFF_TOOLS.has("request_approval"));
  const out = await run(tool, { action: "  Deploy  ", why: "green", choices: ["a", "", "b", "c", "d", "e"] });
  assert.deepEqual(out, { text: CARD_WAIT_TEXT, endTurn: true });
  assert.deepEqual(sent, [["request_approval", { action: "Deploy", why: "green", choices: ["a", "b", "c", "d"] }]]);
});

test("request_approval: an empty action is refused without calling the app; app errors and silence are plain text", async () => {
  let calls = 0;
  const empty = requestApprovalTool({ call: async () => { calls++; return { ok: true, result: { card: true } }; } });
  assert.equal((await run(empty, { action: "  ", why: "x" })).isError, true);
  assert.equal(calls, 0);
  const refused = requestApprovalTool({ call: async () => ({ ok: false, error: { code: "TOOL_NOT_ALLOWED", message: "Not in a group." } }) });
  const out = await run(refused, { action: "x", why: "y" });
  assert.equal(out.isError, true);
  assert.equal(out.endTurn, undefined, "a refused card does not end the turn");
  assert.match(out.text, /Not in a group/);
  const throws = requestApprovalTool({ call: async () => { throw new Error("gone"); } });
  assert.match((await run(throws, { action: "x", why: "y" })).text, /did not answer/);
  const nocard = requestApprovalTool({ call: async () => ({ ok: true, result: {} }) as SandboxReply });
  assert.equal((await run(nocard, { action: "x", why: "y" })).endTurn, undefined);
});

test("ask_user keeps its text form with no app, no choices or another surface", async () => {
  assert.deepEqual(await run(askUserTool(), { question: "Which?", choices: ["a", "b"] }), { text: "Which?\n(a / b)", endTurn: true });
  assert.deepEqual(await run(askUserTool(), { question: "Which?" }), { text: "Which?", endTurn: true });
  assert.equal((await run(askUserTool(), { question: "  " })).text, "Write the question first.");
  let calls = 0;
  const call = async (): Promise<SandboxReply> => { calls++; return { ok: true, result: { card: true } }; };
  assert.equal((await run(askUserTool({ call, inAppDm: false }), { question: "Which?", choices: ["a"] })).text, "Which?\n(a)");
  assert.equal((await run(askUserTool({ call, inAppDm: true }), { question: "Which?" })).text, "Which?");
  assert.equal(calls, 0, "the app is not asked");
  assert.deepEqual(await run(askUserTool({ call, inAppDm: true }), { question: "Which?", choices: ["a", "b"] }), { text: CARD_WAIT_TEXT, endTurn: true });
  assert.equal(calls, 1);
});

test("ask_user falls back to text when the app errors or throws", async () => {
  const bad = askUserTool({ call: async () => ({ ok: false, error: { code: "X", message: "no" } }), inAppDm: true });
  assert.equal((await run(bad, { question: "Which?", choices: ["a", "b"] })).text, "Which?\n(a / b)");
  const boom = askUserTool({ call: async () => { throw new Error("gone"); }, inAppDm: true });
  assert.equal((await run(boom, { question: "Which?", choices: ["a", "b"] })).text, "Which?\n(a / b)");
});

test("the job note asks for quiet between turns and names request_approval and the app's own gate", () => {
  assert.match(JOB_TURN_NOTE, /keep your owner in the loop/);
  assert.doesNotMatch(JOB_TURN_NOTE, /stay quiet/);
  assert.match(JOB_TURN_NOTE, /request_approval/);
  assert.match(JOB_TURN_NOTE, /never say it was approved/);
  assert.match(JOB_TURN_NOTE, /do not try to get around that/);
});

test("the job step line says the app goes on by itself; the person line keeps say continue", () => {
  assert.doesNotMatch(jobTurnStepsText(60), /continue/i);
  assert.match(stoppedAfterStepsText(60), /Say continue/);
});
