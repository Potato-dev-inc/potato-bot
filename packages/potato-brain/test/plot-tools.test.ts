import { test } from "node:test";
import assert from "node:assert/strict";
import { checkPlotArgs, plotTool, PLOT_MAX_POINTS, PLOT_TITLE_MAX } from "../src/core/plot-tools.ts";
import type { SandboxReply } from "../src/core/sandbox-tools.ts";

const CTX = { turn: 1, where: undefined, model: "test" };
const pts = (n: number) => Array.from({ length: n }, (_, i) => ({ label: `p${i}`, value: i }));

test("render_plot is an owner tool; bad input never reaches the app and gets a plain sentence", async () => {
  const calls: Array<Record<string, unknown>> = [];
  const tool = plotTool({ call: async (_n, a) => { calls.push(a); return { ok: true, result: { note: "In the chat." } } as SandboxReply; } });
  assert.equal(tool.gate, "owner");
  const bad: Array<Record<string, unknown>> = [
    { type: "donut", points: pts(2) },
    { type: "bar" },
    { type: "bar", points: [] },
    { type: "bar", points: pts(PLOT_MAX_POINTS + 1) },
    { type: "pie", points: pts(13) },
    { type: "line", points: pts(1) },
    { type: "bar", title: "x".repeat(PLOT_TITLE_MAX + 1), points: pts(2) },
    { type: "bar", points: [{ label: "a", value: "7" }] },
    { type: "bar", points: [{ label: "a", value: Number.NaN }] },
    { type: "bar", points: [{ label: "a", value: 1e13 }] },
    { type: "bar", points: [{ label: "", value: 1 }] },
    { type: "bar", points: [{ label: "y".repeat(41), value: 1 }] },
    { type: "pie", points: [{ label: "a", value: -1 }, { label: "b", value: 3 }] },
    { type: "pie", points: [{ label: "a", value: 0 }] },
  ];
  for (const args of bad) {
    const out = await tool.run(args, CTX);
    assert.ok(typeof out !== "string" && out.isError === true, JSON.stringify(args).slice(0, 80));
  }
  assert.deepEqual(calls, []);
  assert.equal(checkPlotArgs({ type: "line", title: "Trend", points: pts(PLOT_MAX_POINTS) }), null);
  const done = await tool.run({ type: "bar", title: "Totals", points: pts(3) }, CTX);
  assert.ok(typeof done !== "string" && done.isError === undefined && done.text === "In the chat.");
  assert.equal(calls.length, 1);
});

test("a refusal from the app is an error the model can read", async () => {
  const tool = plotTool({ call: async () => ({ ok: false, error: { code: "POTATO_PLOT_WHERE", message: "Only in your private chat." } }) });
  const out = await tool.run({ type: "bar", points: pts(2) }, CTX);
  assert.ok(typeof out !== "string" && out.isError === true && /Only in your private chat/.test(out.text));
});
