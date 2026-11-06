import { test } from "node:test";
import assert from "node:assert/strict";
import { formatJobReply, jobTools, JOB_SPECS, JOB_TOOL_NAMES } from "../src/core/job-tools.ts";
import { ROOM_OFF_TOOLS } from "../src/core/loop.ts";

const ctx = { turn: 1, where: undefined, model: "m" };
const textOf = (out: string | { text: string }): string => (typeof out === "string" ? out : out.text);

test("the three job tools are owner-person gated, named as the app expects, and off in a room", () => {
  const tools = jobTools({ call: async () => ({ ok: true, result: {} }) });
  assert.deepEqual(tools.map((t) => t.spec.name), ["job_start", "job_log", "job_done"]);
  assert.ok(tools.every((t) => t.gate === "owner_person"), "never a routine's run");
  for (const n of JOB_TOOL_NAMES) assert.ok(ROOM_OFF_TOOLS.has(n), `${n} is in ROOM_OFF_TOOLS`);
});

test("each tool sends its own name and arguments to the app, and the answer is one plain line", async () => {
  const sent: Array<[string, Record<string, unknown>]> = [];
  const tools = jobTools({
    call: async (name, args) => {
      sent.push([name, args]);
      return { ok: true, result: name === "job_start" ? { note: "The job is saved." } : {} };
    },
  });
  assert.equal(textOf(await tools[0]!.run({ goal: "Write the report" }, ctx)), "The job is started. The job is saved.");
  assert.equal(textOf(await tools[1]!.run({ step: "Outline" }, ctx)), "Step logged.");
  assert.equal(textOf(await tools[2]!.run({ summary: "Done" }, ctx)), "The job is marked done.");
  assert.deepEqual(sent, [["job_start", { goal: "Write the report" }], ["job_log", { step: "Outline" }], ["job_done", { summary: "Done" }]]);
});

test("an app error is a plain sentence with the code; a call that fails in transit never throws", async () => {
  const out = formatJobReply("job_start", { ok: false, error: { code: "POTATO_JOB_EXISTS", message: "A job is already running." } });
  assert.equal(out.isError, true);
  assert.equal(out.text, "job_start did not run [POTATO_JOB_EXISTS]: A job is already running.");
  const tools = jobTools({ call: async () => { throw new Error("gone"); } });
  assert.match(textOf(await tools[1]!.run({ step: "x" }, ctx)), /did not run \[JOB_FAILED\]/);
});

test("the descriptions say what the app will do, with the limits", () => {
  assert.match(JOB_SPECS.job_start.description, /starts your next turn by itself/);
  assert.match(JOB_SPECS.job_start.description, /4 hours or 5 dollars/);
  assert.match(JOB_SPECS.job_log.description, /three in a row stop the job/);
});
