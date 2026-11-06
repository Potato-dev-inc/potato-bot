// Live steering: a person's message that arrives while a turn runs is read between tool rounds, never starts a second
// turn, and is never lost when the turn ends first.
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
import type { ChatRequest, ChatResult, Provider } from "../src/types.ts";
import { startMockBridge } from "./helpers/mock-bridge.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const usage = { promptTokens: 10, completionTokens: 5, cacheHitTokens: 0, cacheMissTokens: 10 };

/** Each chat call waits for its gate (if any), then answers from the script. Counts how many run at once. */
class Gated implements Provider {
  readonly name = "gated";
  calls: ChatRequest[] = [];
  running = 0;
  maxRunning = 0;
  gates: Array<Promise<void> | undefined> = [];
  private script: Array<(req: ChatRequest) => ChatResult>;
  constructor(script: Array<(req: ChatRequest) => ChatResult>) {
    this.script = script;
  }
  async chat(req: ChatRequest): Promise<ChatResult> {
    const n = this.calls.length;
    this.calls.push(structuredClone({ ...req, signal: undefined }));
    this.running++;
    this.maxRunning = Math.max(this.maxRunning, this.running);
    try {
      await this.gates[n];
      const step = this.script[n];
      return step ? step(req) : { text: "ok", usage, model: "g" };
    } finally {
      this.running--;
    }
  }
}

const note = (text: string): ChatResult => ({
  text: "",
  usage,
  model: "g",
  toolCalls: [{ id: "call-0", name: "message_user", arguments: JSON.stringify({ text }) }],
});
const say = (text: string): ChatResult => ({ text, usage, model: "g" });

async function rig(provider: Provider) {
  const root = mkdtempSync(join(tmpdir(), "potato-steer-"));
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
  });
  void channel.start();
  return {
    bridge,
    async done() {
      await channel.stop();
      await bridge.close();
      rmSync(root, { recursive: true, force: true });
    },
  };
}

const send = (id: string, text: string, over: Record<string, unknown> = {}): Record<string, unknown> => ({
  type: "send",
  id,
  conversationId: "app:dm:bot1",
  surface: "app",
  from: { id: "owner", name: "Pat", trust: "owner_pc" },
  text,
  tools: "full",
  ...over,
});

const pause = (ms = 150): Promise<void> => new Promise((r) => setTimeout(r, ms));
function gate(): { promise: Promise<void>; open: () => void } {
  let open!: () => void;
  const promise = new Promise<void>((r) => (open = r));
  return { promise, open };
}
const userTexts = (req: ChatRequest): string[] => req.messages.filter((m) => m.role === "user").map((m) => String(m.content));

test("a message written during a turn is read at the next round and starts no second turn", async () => {
  const g = gate();
  const p = new Gated([() => note("working"), () => note("status: step 1 of 3"), () => say("all done")]);
  p.gates[1] = g.promise;
  const r = await rig(p);
  try {
    r.bridge.push(send("m1", "do the big thing"));
    await r.bridge.waitFor((e) => e["type"] === "reply" && e["note"] === true);
    r.bridge.push(send("m2", "status?"));
    await pause();
    g.open();
    await r.bridge.waitFor((e) => e["type"] === "typing" && e["on"] === false);
    const picked = r.bridge.events.filter((e) => e["type"] === "picked");
    assert.deepEqual(picked.map((e) => e["ids"]), [["m2"]]);
    assert.ok(!userTexts(p.calls[1] as ChatRequest).some((t) => t.includes("status?")));
    assert.ok(userTexts(p.calls[2] as ChatRequest).some((t) => t.includes("status?") && t.includes("while you were working")));
    assert.equal(r.bridge.events.filter((e) => e["type"] === "typing" && e["on"] === true).length, 1, "one turn");
    const replies = r.bridge.events.filter((e) => e["type"] === "reply");
    assert.deepEqual(replies.map((e) => [e["note"] === true, e["inReplyTo"]]), [[true, "m1"], [true, "m1"], [false, "m1"]]);
    assert.equal(p.maxRunning, 1, "never two model calls at once");
  } finally {
    await r.done();
  }
});

test("several messages are picked up together, oldest first", async () => {
  const g = gate();
  const p = new Gated([() => note("working"), () => note("again"), () => say("ok")]);
  p.gates[1] = g.promise;
  const r = await rig(p);
  try {
    r.bridge.push(send("m1", "start"));
    await r.bridge.waitFor((e) => e["type"] === "reply" && e["note"] === true);
    r.bridge.push(send("m2", "first extra"), send("m3", "second extra"));
    await pause();
    g.open();
    await r.bridge.waitFor((e) => e["type"] === "typing" && e["on"] === false);
    assert.deepEqual(r.bridge.events.filter((e) => e["type"] === "picked").map((e) => e["ids"]), [["m2", "m3"]]);
    const seen = userTexts(p.calls[2] as ChatRequest).find((t) => t.includes("first extra")) ?? "";
    assert.ok(seen.indexOf("first extra") < seen.indexOf("second extra"));
  } finally {
    await r.done();
  }
});

test("a message that arrives as the turn ends is not lost: it runs next, in order", async () => {
  const g = gate();
  const p = new Gated([() => say("first answer"), () => say("second answer")]);
  p.gates[0] = g.promise;
  const r = await rig(p);
  try {
    r.bridge.push(send("m1", "hello"));
    await pause();
    r.bridge.push(send("m2", "and one more thing"));
    await pause();
    g.open();
    await r.bridge.waitFor((e) => e["type"] === "reply" && e["inReplyTo"] === "m2");
    const replies = r.bridge.events.filter((e) => e["type"] === "reply" && e["note"] !== true);
    assert.deepEqual(replies.map((e) => e["inReplyTo"]), ["m1", "m2"]);
    assert.deepEqual(r.bridge.events.filter((e) => e["type"] === "picked").map((e) => e["ids"]), [["m2"]]);
    assert.equal(p.maxRunning, 1);
    assert.ok(userTexts(p.calls[1] as ChatRequest).some((t) => t.includes("and one more thing")));
  } finally {
    await r.done();
  }
});

test("a routine, a stranger and a job turn are never read into another turn", async () => {
  const g = gate();
  const p = new Gated([() => say("a"), () => say("b"), () => say("c"), () => say("d")]);
  p.gates[0] = g.promise;
  const r = await rig(p);
  try {
    r.bridge.push(send("m1", "hello"));
    await pause();
    r.bridge.push(
      send("r1", "routine words", { from: { id: "routine", name: "routine", trust: "owner_pc" } }),
      send("s1", "stranger words", { from: { id: "x", name: "X", trust: "others" } }),
      send("j1", "Keep working on your job.", { context: "<job_data>goal</job_data>" }),
    );
    await pause();
    g.open();
    await r.bridge.waitFor((e) => e["type"] === "reply" && e["inReplyTo"] === "j1");
    assert.equal(r.bridge.events.filter((e) => e["type"] === "picked").length, 0, "none was steered");
    const replies = r.bridge.events.filter((e) => e["type"] === "reply" && e["note"] !== true);
    assert.deepEqual(replies.map((e) => e["inReplyTo"]), ["m1", "r1", "s1", "j1"]);
  } finally {
    await r.done();
  }
});
