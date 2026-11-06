import { test } from "node:test";
import assert from "node:assert/strict";
import { addMcpServerTool, ADD_MCP_SERVER_SPEC } from "../src/core/mcp-tools.ts";
import { ROOM_OFF_TOOLS } from "../src/core/loop.ts";

const FAKE_TOKEN = "fake-token-77c0e1b2a9";
const ctx = { turn: 1, where: undefined, model: "m" };
type Out = string | { text: string; endTurn?: boolean; isError?: boolean };
const run = async (tool: { run: (a: Record<string, unknown>, c: typeof ctx) => Out | Promise<Out> }, args: Record<string, unknown>) => {
  const out = await tool.run(args, ctx);
  return typeof out === "string" ? { text: out } : out;
};

function make() {
  const sent: Array<Record<string, unknown>> = [];
  const tool = addMcpServerTool({
    call: async (_name, args) => {
      sent.push(args);
      return { ok: true, result: { card: true, cardId: "card-1" } };
    },
  });
  return { sent, tool };
}

test("a link goes to the app as a name, the link and a reason only, and the turn ends on the card", async () => {
  const { sent, tool } = make();
  const out = await run(tool, { name: "tracker", url: " https://mcp.example.com/mcp ", why: "to read issues" });
  assert.equal(out.endTurn, true);
  assert.deepEqual(sent, [{ name: "tracker", url: "https://mcp.example.com/mcp", why: "to read issues" }]);
  assert.equal(tool.gate, "owner_person");
  assert.ok(ROOM_OFF_TOOLS.has("add_mcp_server"), "off in a room");
});

test("a secret in any argument refuses the call before the app hears of it", async () => {
  for (const extra of ["token", "apiKey", "api_key", "headers", "authorization", "env", "password", "bearer", "Token"]) {
    const { sent, tool } = make();
    const out = await run(tool, { name: "tracker", url: "https://mcp.example.com/mcp", [extra]: FAKE_TOKEN });
    assert.equal(out.isError, true, extra);
    assert.equal(sent.length, 0, extra);
    assert.equal(out.text.includes(FAKE_TOKEN), false);
  }
  const { sent, tool } = make();
  const out = await run(tool, { name: "files", command: "npx", env: { KEY: FAKE_TOKEN } });
  assert.equal(out.isError, true);
  assert.equal(sent.length, 0);
});

test("a command and a link together are refused; a plain command still goes straight through as before", async () => {
  const both = make();
  assert.equal((await run(both.tool, { name: "x", command: "npx", url: "https://mcp.example.com/mcp" })).isError, true);
  assert.equal(both.sent.length, 0);
  const plain = make();
  const out = await run(plain.tool, { name: "files", command: "npx", args: ["-y", "pkg"] });
  assert.deepEqual(plain.sent, [{ name: "files", command: "npx", args: ["-y", "pkg"] }]);
  assert.equal(out.endTurn, undefined);
});

test("the app's refusal of a link is plain text and no card is claimed", async () => {
  const tool = addMcpServerTool({ call: async () => ({ ok: false, error: { code: "MCP_URL_PRIVATE", message: "That address is on a private network." } }) });
  const out = await run(tool, { name: "x", url: "https://10.0.0.1/mcp" });
  assert.equal(out.isError, true);
  assert.equal(out.endTurn, undefined);
  assert.match(out.text, /private network/);
});

test("the tool says a link needs the owner's card, takes no secret and points to Connectors", () => {
  assert.match(ADD_MCP_SERVER_SPEC.description, /card/);
  assert.match(ADD_MCP_SERVER_SPEC.description, /Add it/);
  assert.match(ADD_MCP_SERVER_SPEC.description, /no secret/);
  assert.match(ADD_MCP_SERVER_SPEC.description, /Settings > Connectors/);
  assert.deepEqual(ADD_MCP_SERVER_SPEC.parameters.required, ["name"]);
});
