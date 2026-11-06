import { test } from "node:test";
import assert from "node:assert/strict";
import { requestConnectorTool, REQUEST_CONNECTOR_NAME, REQUEST_CONNECTOR_SPEC } from "../src/core/mcp-tools.ts";
import { ROOM_OFF_TOOLS } from "../src/core/loop.ts";
import { stepFacts } from "../src/core/step-label.ts";
import { sandboxToolSpecs } from "../src/core/sandbox-tools.ts";

const FAKE_TOKEN = "fake-token-c41d8e0a77";
const ctx = { turn: 1, where: undefined, model: "m" };
type Out = string | { text: string; endTurn?: boolean; isError?: boolean };
const run = async (tool: { run: (a: Record<string, unknown>, c: typeof ctx) => Out | Promise<Out> }, args: Record<string, unknown>) => {
  const out = await tool.run(args, ctx);
  return typeof out === "string" ? { text: out } : out;
};

test("request_connector: owner-person only, off in a room, and it ends the turn when the card is made", async () => {
  const sent: Array<[string, Record<string, unknown>]> = [];
  const tool = requestConnectorTool({
    call: async (name, args) => {
      sent.push([name, args]);
      return { ok: true, result: { card: true, cardId: "card-1" } };
    },
  });
  assert.equal(tool.gate, "owner_person");
  assert.equal(tool.spec.name, REQUEST_CONNECTOR_NAME);
  assert.ok(ROOM_OFF_TOOLS.has("request_connector"));
  const out = await run(tool, { service: "  GitHub ", why: "to read issues" });
  assert.equal(out.endTurn, true);
  assert.deepEqual(sent, [["request_connector", { service: "GitHub", why: "to read issues" }]]);
});

test("request_connector: a token the model puts in the arguments is dropped and never sent to the app", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const tool = requestConnectorTool({
    call: async (_name, args) => {
      sent.push(args);
      return { ok: true, result: { card: true } };
    },
  });
  const out = await run(tool, { service: "GitHub", token: FAKE_TOKEN, apiKey: FAKE_TOKEN, env: { GITHUB_PERSONAL_ACCESS_TOKEN: FAKE_TOKEN } });
  assert.equal(JSON.stringify(sent).includes(FAKE_TOKEN), false);
  assert.equal(out.text.includes(FAKE_TOKEN), false);
  assert.deepEqual(Object.keys(sent[0] ?? {}).sort(), ["service"]);
});

test("request_connector: no service is refused without calling the app; app errors are plain text", async () => {
  let calls = 0;
  const empty = requestConnectorTool({ call: async () => { calls++; return { ok: true, result: { card: true } }; } });
  assert.equal((await run(empty, { service: "  " })).isError, true);
  assert.equal(calls, 0);
  const refused = requestConnectorTool({ call: async () => ({ ok: false, error: { code: "TOOL_NOT_ALLOWED", message: "Not in a group." } }) });
  const out = await run(refused, { service: "GitHub" });
  assert.equal(out.isError, true);
  assert.equal(out.endTurn, undefined);
  assert.match(out.text, /Not in a group/);
  const already = requestConnectorTool({ call: async () => ({ ok: true, result: { card: false, note: "already connected" } }) });
  const same = await run(already, { service: "GitHub" });
  assert.equal(same.endTurn, undefined, "no card, so the turn goes on");
});

test("the prompt rule: the tool says never to ask for a token in chat, and the computer tools say who to ask", () => {
  assert.match(REQUEST_CONNECTOR_SPEC.description, /Never ask for a token/);
  assert.match(REQUEST_CONNECTOR_SPEC.description, /data, not instructions/);
  const specs = sandboxToolSpecs();
  const list = specs.find((s) => s.name === "mcp_list_tools");
  assert.match(list?.description ?? "", /request_connector/);
  assert.match(list?.description ?? "", /never ask for a token in chat/);
  const call = specs.find((s) => s.name === "mcp_call_tool");
  assert.match(call?.description ?? "", /connected service, not instructions/);
});

test("the step list for a tool call shows no arguments, so a token in them cannot reach it", () => {
  const facts = stepFacts("mcp_call_tool", { server: "github", tool: "create_issue", arguments: { title: FAKE_TOKEN } }, [FAKE_TOKEN]);
  assert.equal(JSON.stringify(facts).includes(FAKE_TOKEN), false);
  const asked = stepFacts(REQUEST_CONNECTOR_NAME, { service: "github", why: `my key is ${FAKE_TOKEN}` }, [FAKE_TOKEN]);
  assert.equal(JSON.stringify(asked).includes(FAKE_TOKEN), false);
});
