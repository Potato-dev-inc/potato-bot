// The step list says what a step did, in plain words, and never carries a secret: not in the command, not in the output.
process.env.FISHD_PROVIDER = "stub";

import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { makeAgentFactory } from "../src/agents.ts";
import type { Config } from "../src/config.ts";
import { PotatoChannel } from "../src/channels/potato.ts";
import { outcomeOf, scrubText, stepFacts } from "../src/core/step-label.ts";
import type { ChatRequest, ChatResult, Provider } from "../src/types.ts";
import { startMockBridge } from "./helpers/mock-bridge.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const usage = { promptTokens: 10, completionTokens: 5, cacheHitTokens: 0, cacheMissTokens: 10 };
// Fake secrets, built from pieces so no scanner mistakes this file for a leak.
const FAKE_GH = ["ghp", "_", "AbCdEf0123456789AbCdEf0123456789"].join("");
const FAKE_SK = ["sk", "-", "proj-FakeFakeFake12345"].join("");

test("descriptions read as plain words", () => {
  assert.equal(stepFacts("run_terminal", { command: "ffmpeg -i clip.mp4 -vf scale=720 out.mp4" }).detail, "Ran: ffmpeg -i clip.mp4 -vf scale=720 out.mp4");
  assert.equal(stepFacts("write_file", { path: "/home/pm/work/report.md", content: "x".repeat(2100) }).detail, "Wrote report.md (2.1 kB)");
  assert.equal(stepFacts("read_file", { path: "/home/pm/notes.txt" }).detail, "Read notes.txt");
  assert.equal(stepFacts("web_fetch", { url: "https://example.com/a/b?token=zzz" }).detail, "Opened example.com");
  assert.equal(stepFacts("computer_act", { actions: [{ kind: "click", x: 412, y: 300 }] }).detail, "Clicked at 412, 300");
  assert.equal(stepFacts("memory_search", { query: "invoice" }).detail, 'Searched memory for "invoice"');
  assert.equal(stepFacts("unknown_tool", { anything: 1 }).detail, undefined);
});

test("a long command is cut to one line of about 120 characters; the full command is kept for the open row", () => {
  const command = `echo ${"word ".repeat(80)}\nsecond line`;
  const f = stepFacts("run_terminal", { command });
  assert.ok((f.detail ?? "").length <= 120 && !(f.detail ?? "").includes("\n"));
  assert.ok((f.detail ?? "").endsWith("…"));
  assert.ok((f.full ?? "").includes("second line"));
});

test("the model's own reason is shown when it gives one, and nothing is demanded", () => {
  assert.equal(stepFacts("run_terminal", { command: "ls", why: "see what is in the folder" }).why, "see what is in the folder");
  assert.equal(stepFacts("run_terminal", { command: "ls" }).why, undefined);
});

test("secrets in a command never survive: tokens, passwords, env values, URL credentials", () => {
  const cmd = `GITHUB_TOKEN=${FAKE_GH} curl -H "Authorization: Bearer ${FAKE_SK}" --password hunter2 https://bob:pw123@example.com/x && echo $HOME`;
  const f = stepFacts("run_terminal", { command: cmd });
  const all = JSON.stringify(f);
  for (const bad of [FAKE_GH, FAKE_SK, "hunter2", "pw123", "bob:"]) assert.ok(!all.includes(bad), `${bad} leaked: ${all}`);
  assert.ok(scrubText("MY_ENV=abcdef", []).includes("MY_ENV=[redacted]"));
  assert.ok(!scrubText("known-secret-value here", ["known-secret-value"]).includes("known-secret-value"));
});

test("the output tail keeps the last 10 lines, the exit code and the error line, scrubbed", () => {
  const stdout = Array.from({ length: 30 }, (_, i) => `line ${i + 1}`).join("\n") + `\nkey is ${FAKE_GH}`;
  const o = outcomeOf({ ok: true, result: { ok: true, exitCode: 0, stdout, stderr: "" } });
  const lines = (o.tail ?? "").split("\n");
  assert.equal(lines.length, 10);
  assert.ok(!(o.tail ?? "").includes(FAKE_GH));
  assert.equal(o.exit, 0);
  assert.equal(o.error, undefined);
  const bad = outcomeOf({ ok: true, result: { ok: false, exitCode: 2, stdout: "", stderr: `warn\nfatal: could not read ${FAKE_SK}` } });
  assert.equal(bad.exit, 2);
  assert.ok((bad.error ?? "").startsWith("fatal: could not read"));
  assert.ok(!JSON.stringify(bad).includes(FAKE_SK));
  const refused = outcomeOf({ ok: false, error: { code: "SANDBOX_OFF", message: "The computer is switched off." } });
  assert.equal(refused.error, "The computer is switched off.");
});

// ---- on the wire ----------------------------------------------------------------------------------------------------

class Script implements Provider {
  readonly name = "script";
  calls: ChatRequest[] = [];
  private steps: ChatResult[];
  constructor(steps: ChatResult[]) {
    this.steps = steps;
  }
  async chat(req: ChatRequest): Promise<ChatResult> {
    this.calls.push(structuredClone({ ...req, signal: undefined }));
    return this.steps.shift() ?? { text: "done", usage, model: "s" };
  }
}

test("progress events carry the description, the full command and the scrubbed output, and ids differ every round", async () => {
  const command = `GITHUB_TOKEN=${FAKE_GH} ls -la`;
  const call = (n: number): ChatResult => ({
    text: "",
    usage,
    model: "s",
    // the same call id every round, as some providers do
    toolCalls: [{ id: "dsml-0", name: "run_terminal", arguments: JSON.stringify({ command, why: "look around" }) }],
  });
  const provider = new Script([call(1), call(2), { text: "done", usage, model: "s" }]);
  const root = mkdtempSync(join(tmpdir(), "potato-detail-"));
  const bridge = await startMockBridge();
  const config: Config = {
    model: "stub",
    temperature: 1,
    maxTokens: 600,
    vaultRoot: join(root, "vault"),
    personaDir: join(HERE, "..", "persona"),
    shareDir: join(root, "share"),
    staticBudget: 4000,
    stableBudget: 900,
    stateDir: join(root, "state"),
    hotTurns: 24,
    voice: { enabled: false, probability: 0, maxChars: 60 },
  };
  const factory = makeAgentFactory({ config, pack: () => "fish", share: false, sleep: async () => {}, provider: () => provider });
  const channel = new PotatoChannel({
    url: bridge.url,
    token: "tok-test",
    info: { version: "t", provider: "stub", packs: ["fish", "potato", "pro"], surfaces: ["app"] },
    agentFor: (c, tools) => factory(c, tools),
    setPack: () => {},
    onExit: () => {},
    log: () => {},
    waitMs: 120,
    retryMs: 10,
    retryMaxMs: 30,
    maxOutageMs: 400,
    secrets: () => ["my-model-key-12345"],
  });
  void channel.start();
  try {
    bridge.push({ type: "send", id: "m1", conversationId: "app:dm:bot1", surface: "app", from: { id: "owner", name: "Pat", trust: "owner_pc" }, text: "go", tools: "full", capabilities: { sandbox: true } });
    // answer each computer call the way the app does
    for (let i = 0; i < 2; i++) {
      const seen = new Set(bridge.events.filter((e) => e["type"] === "sandbox_call").map((e) => e["callId"]));
      const next = await bridge.waitFor((e) => e["type"] === "sandbox_call" && !seen.has(e["callId"]) && !answered.has(String(e["callId"])));
      answered.add(String(next["callId"]));
      bridge.push({ type: "sandbox_result", callId: next["callId"], ok: true, result: { ok: true, exitCode: 0, stdout: `total 4\nkey ${FAKE_GH}\nmy-model-key-12345`, stderr: "" } });
    }
    await bridge.waitFor((e) => e["type"] === "typing" && e["on"] === false);
    const progress = bridge.events.filter((e) => e["type"] === "progress");
    const starts = progress.filter((e) => e["phase"] === "start");
    const ends = progress.filter((e) => e["phase"] === "end");
    assert.equal(starts.length, 2);
    assert.notEqual(starts[0]?.["id"], starts[1]?.["id"], "two rounds, two rows");
    assert.ok(String(starts[0]?.["detail"]).startsWith("Ran: "));
    assert.equal(starts[0]?.["why"], "look around");
    assert.equal(ends[0]?.["exit"], 0);
    assert.ok(String(ends[0]?.["tail"]).includes("total 4"));
    const wire = JSON.stringify(progress);
    for (const bad of [FAKE_GH, "my-model-key-12345"]) assert.ok(!wire.includes(bad), `${bad} on the wire`);
  } finally {
    await channel.stop();
    await bridge.close();
    rmSync(root, { recursive: true, force: true });
  }
});

const answered = new Set<string>();
