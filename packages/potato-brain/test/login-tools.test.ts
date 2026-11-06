import { test } from "node:test";
import assert from "node:assert/strict";
import { requestLoginTool, REQUEST_LOGIN_NAME, REQUEST_LOGIN_SPEC } from "../src/core/login-tools.ts";
import { ROOM_OFF_TOOLS } from "../src/core/loop.ts";
import { stepFacts } from "../src/core/step-label.ts";
import { sandboxToolSpecs, SANDBOX_TURN_NOTE } from "../src/core/sandbox-tools.ts";

const FAKE_PASSWORD = "fake-pw-Zq81x-91";
const ctx = { turn: 1, where: undefined, model: "m" };
type Out = string | { text: string; endTurn?: boolean; isError?: boolean };
const run = async (tool: { run: (a: Record<string, unknown>, c: typeof ctx) => Out | Promise<Out> }, args: Record<string, unknown>) => {
  const out = await tool.run(args, ctx);
  return typeof out === "string" ? { text: out } : out;
};

test("request_login: owner-person only, off in a room, and it ends the turn when the card is made", async () => {
  const sent: Array<[string, Record<string, unknown>]> = [];
  const tool = requestLoginTool({
    call: async (name, args) => {
      sent.push([name, args]);
      return { ok: true, result: { card: true, cardId: "card-1" } };
    },
  });
  assert.equal(tool.gate, "owner_person");
  assert.equal(tool.spec.name, REQUEST_LOGIN_NAME);
  assert.ok(ROOM_OFF_TOOLS.has("request_login"));
  const out = await run(tool, { site: "  github.com ", why: "to read your issues" });
  assert.equal(out.endTurn, true);
  assert.deepEqual(sent, [["request_login", { site: "github.com", why: "to read your issues" }]]);
});

test("request_login: a password, user name, code or cookie in the arguments is refused and never sent to the app", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const tool = requestLoginTool({
    call: async (_name, args) => {
      sent.push(args);
      return { ok: true, result: { card: true } };
    },
  });
  for (const args of [
    { site: "github.com", password: FAKE_PASSWORD },
    { site: "github.com", pass: FAKE_PASSWORD },
    { site: "github.com", username: "me", why: "x" },
    { site: "github.com", otp: FAKE_PASSWORD },
    { site: "github.com", extra: { cookie: FAKE_PASSWORD } },
  ]) {
    const out = await run(tool, args);
    assert.equal(out.isError, true);
    assert.equal(out.endTurn, undefined);
    assert.equal(out.text.includes(FAKE_PASSWORD), false, "the refusal does not echo it");
  }
  assert.equal(sent.length, 0, "nothing reached the app");
});

test("request_login: a site that carries a secret or is not a plain site name is refused without calling the app", async () => {
  let calls = 0;
  const tool = requestLoginTool({ call: async () => { calls++; return { ok: true, result: { card: true } }; } });
  for (const site of ["", "   ", `https://me:${FAKE_PASSWORD}@github.com`, `github.com/login?password=${FAKE_PASSWORD}`, "two words.com", "x".repeat(200), 42 as unknown as string]) {
    const out = await run(tool, { site });
    assert.equal(out.isError, true, String(site));
    assert.equal(out.text.includes(FAKE_PASSWORD), false);
  }
  assert.equal(calls, 0);
});

test("request_login: app errors are plain text and a no-card answer does not end the turn", async () => {
  const refused = requestLoginTool({ call: async () => ({ ok: false, error: { code: "TOOL_NOT_ALLOWED", message: "Not in a group." } }) });
  const out = await run(refused, { site: "github.com" });
  assert.equal(out.isError, true);
  assert.equal(out.endTurn, undefined);
  assert.match(out.text, /Not in a group/);
  const none = requestLoginTool({ call: async () => ({ ok: true, result: { card: false } }) });
  assert.equal((await run(none, { site: "github.com" })).endTurn, undefined);
  const down = requestLoginTool({ call: async () => { throw new Error("gone"); } });
  assert.equal((await run(down, { site: "github.com" })).isError, true);
});

test("the prompt rule: never ask for a password in chat, use request_login", () => {
  assert.match(REQUEST_LOGIN_SPEC.description, /Never ask for a password/);
  assert.match(REQUEST_LOGIN_SPEC.description, /you never see/i);
  assert.match(REQUEST_LOGIN_SPEC.description, /act as your owner|act as them/i);
  const rules = SANDBOX_TURN_NOTE;
  assert.match(rules, /request_login/);
  assert.match(rules, /never ask for a password/i);
  const ask = sandboxToolSpecs().find((s) => s.name === "ask_person_for_screen");
  assert.match(ask?.description ?? "", /request_login/);
});

test("the step list for request_login shows no arguments, so nothing typed into them can reach it", () => {
  const facts = stepFacts(REQUEST_LOGIN_NAME, { site: "github.com", why: `my password is ${FAKE_PASSWORD}` }, [FAKE_PASSWORD]);
  assert.equal(JSON.stringify(facts).includes(FAKE_PASSWORD), false);
});
