// A DeepSeek build can print a tool call into the text instead of `tool_calls`. The brain reads that block
// (DSML), runs the call, and never shows the markup to the person.
import { test } from "node:test";
import assert from "node:assert/strict";
import { parseTextToolCalls } from "../src/core/openai.ts";

const fullwidth = (s: string) => s.replaceAll("|", "\uFF5C");

test("a DSML block becomes the tool call and the text keeps the words around it", () => {
  const raw = fullwidth(`Let me look.\n<||DSML|| calls>\n<||DSML|| invoke name="run_terminal">\n<||DSML|| parameter name="command" string="true">ls -la</||DSML|| parameter>\n<||DSML|| parameter name="timeout_ms" string="false">5000</||DSML|| parameter>\n</||DSML|| invoke>\n</||DSML|| calls>`);
  const out = parseTextToolCalls(raw);
  assert.equal(out.calls.length, 1);
  assert.equal(out.calls[0]?.name, "run_terminal");
  assert.deepEqual(JSON.parse(out.calls[0]?.arguments ?? "{}"), { command: "ls -la", timeout_ms: 5000 });
  assert.equal(out.text, "Let me look.");
});

test("several calls in one block keep their order", () => {
  const raw = fullwidth(`<||DSML|| calls><||DSML|| invoke name="read_file"><||DSML|| parameter name="path" string="true">a.txt</||DSML|| parameter></||DSML|| invoke><||DSML|| invoke name="list_files"><||DSML|| parameter name="path" string="true">/tmp</||DSML|| parameter></||DSML|| invoke></||DSML|| calls>`);
  const out = parseTextToolCalls(raw);
  assert.deepEqual(out.calls.map((c) => c.name), ["read_file", "list_files"]);
  assert.deepEqual(out.calls.map((c) => JSON.parse(c.arguments)), [{ path: "a.txt" }, { path: "/tmp" }]);
  assert.equal(out.text, "");
});

test("text with no DSML block is left exactly as it was", () => {
  const raw = "Just a sentence about <function_calls> and nothing else.";
  const out = parseTextToolCalls(raw);
  assert.deepEqual(out, { text: raw, calls: [] });
});

test("a malformed or empty block changes nothing", () => {
  for (const raw of ["<||DSML|| calls></||DSML|| calls>", "<||DSML|| calls><||DSML|| invoke name=\"\"></||DSML|| invoke></||DSML|| calls>"]) {
    const out = parseTextToolCalls(raw);
    assert.deepEqual(out.calls, []);
    assert.equal(out.text, raw);
  }
});
