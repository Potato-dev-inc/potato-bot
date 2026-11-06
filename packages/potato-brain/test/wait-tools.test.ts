import { test } from "node:test";
import assert from "node:assert/strict";
import { formatWaitReply, waitTools, WAIT_TOOL_NAMES } from "../src/core/wait-tools.ts";
import { ROOM_OFF_TOOLS } from "../src/core/loop.ts";

const ctx = { turn: 1, where: undefined, model: "m" };

test("the two wait tools are owner-person gated, so a routine run or a woken turn is never offered them", () => {
  const tools = waitTools({ call: async () => ({ ok: true, result: {} }) });
  assert.deepEqual(tools.map((t) => t.spec.name), [...WAIT_TOOL_NAMES]);
  assert.ok(tools.every((t) => t.gate === "owner_person"));
});

test("a group never gets them", () => {
  for (const name of WAIT_TOOL_NAMES) assert.ok(ROOM_OFF_TOOLS.has(name), `${name} is off in a room`);
});

test("wait_until needs a check, and says what a check is and that it ends the turn", () => {
  const wait = waitTools({ call: async () => ({ ok: true, result: {} }) })[0]!;
  assert.deepEqual(wait.spec.parameters["required"], ["check"]);
  const props = Object.keys(wait.spec.parameters["properties"] as object);
  assert.deepEqual(props.sort(), ["check", "contains", "every", "note", "timeout"]);
  assert.match(wait.spec.description, /read-only command/);
  assert.match(wait.spec.description, /exits 0/);
  assert.match(wait.spec.description, /ends your turn/);
  assert.match(wait.spec.description, /at most 5 open waits/);
});

test("a started wait ends the turn with the app's own sentence, and the arguments go to the app unchanged", async () => {
  let seen: { tool: string; args: Record<string, unknown> } | undefined;
  const wait = waitTools({
    call: async (tool, args) => {
      seen = { tool, args };
      return { ok: true, result: { id: "w-1", message: "Waiting for the export. I will look every 2 minutes for up to 1 hour and come back when it is done." } };
    },
  })[0]!;
  const args = { check: "test -f /home/pm/out.zip", every: "2m", timeout: "1h", note: "the export" };
  const out = await wait.run(args, ctx);
  assert.ok(typeof out !== "string");
  assert.equal(out.endTurn, true);
  assert.equal(out.text, "Waiting for the export. I will look every 2 minutes for up to 1 hour and come back when it is done.");
  assert.deepEqual(seen, { tool: "wait_until", args });
});

test("a refused wait is a plain sentence with its code, and the turn goes on", async () => {
  const wait = waitTools({ call: async () => ({ ok: false, error: { code: "POTATO_CHECK_INVALID", message: "A check may only look, never change anything." } }) })[0]!;
  const out = await wait.run({ check: "rm -rf x" }, ctx);
  assert.ok(typeof out !== "string");
  assert.equal(out.isError, true);
  assert.equal(out.endTurn, undefined);
  assert.equal(out.text, "wait_until did not run [POTATO_CHECK_INVALID]: A check may only look, never change anything.");
});

test("a call that fails in transit is a plain failure, never a throw", async () => {
  const wait = waitTools({ call: async () => { throw new Error("gone"); } })[0]!;
  const out = await wait.run({ check: "ls" }, ctx);
  assert.ok(typeof out !== "string");
  assert.match(out.text, /did not run \[WAIT_FAILED\]/);
  assert.equal(out.endTurn, undefined);
});

test("wait_cancel does not end the turn", async () => {
  const cancel = waitTools({ call: async () => ({ ok: true, result: { cancelled: "w-1" } }) })[1]!;
  const out = await cancel.run({ id: "w-1" }, ctx);
  assert.ok(typeof out !== "string");
  assert.equal(out.text, "That wait is cancelled.");
  assert.equal(out.endTurn, undefined);
});
