// Talking while working: the text a model writes beside a tool call reaches the person live, in order, and stays in
// the history; a code nudge after a quiet stretch; caps; job turns talk; both prompts carry the rule.
import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { Agent, type ExtraTool } from "../src/core/loop.ts";
import { loadPersona } from "../src/persona/persona.ts";
import { initialMood } from "../src/persona/mood.ts";
import { JOB_TURN_NOTE } from "../src/core/job-tools.ts";
import { TALK_NUDGE, TALK_MIN_GAP_MS, TALK_TURN_MAX, Talk } from "../src/core/talk.ts";
import type { ChatRequest, ChatResult, Provider } from "../src/types.ts";

const PERSONA_DIR = fileURLToPath(new URL("../persona", import.meta.url));
const usage = { promptTokens: 10, completionTokens: 5, cacheHitTokens: 0, cacheMissTokens: 10 };
const T0 = new Date(2026, 10, 7, 10, 0, 0).getTime();

class Script implements Provider {
  readonly name = "script";
  requests: ChatRequest[] = [];
  private steps: ChatResult[];
  constructor(steps: ChatResult[]) {
    this.steps = steps;
  }
  async chat(req: ChatRequest): Promise<ChatResult> {
    this.requests.push(structuredClone({ ...req, signal: undefined }));
    return this.steps.shift() ?? { text: "done", usage, model: "s" };
  }
}
let n = 0;
const call = (text: string, name = "probe"): ChatResult => ({ text, usage, model: "s", toolCalls: [{ id: `c${++n}`, name, arguments: "{}" }] });
const final = (text: string): ChatResult => ({ text, usage, model: "s" });
const tool = (name: string, run: () => string): ExtraTool => ({ spec: { name, description: "d", parameters: { type: "object", properties: {} } }, run });
const nudged = (p: Script): boolean[] => p.requests.map((q) => JSON.stringify(q.messages).includes(TALK_NUDGE));

function rig(provider: Provider, pack = "pro", clock: { t: number } = { t: T0 }, extra: ExtraTool[] = []) {
  const log: string[] = [];
  const agent = new Agent({
    provider,
    persona: loadPersona(PERSONA_DIR, pack),
    pack: () => pack,
    mood: initialMood(new Date(T0)),
    now: () => new Date(clock.t),
    extraTools: [tool("probe", () => "ok"), ...extra],
    sleep: async () => {},
  });
  const opts = {
    owner: true,
    where: "app:dm:t",
    onNote: (text: string) => (log.push(`say:${text}`), true),
    onStep: (e: { phase: string; tool: string }) => {
      if (e.phase === "start") log.push(`step:${e.tool}`);
    },
  };
  return { agent, log, opts };
}

test("tool-round text is posted live, in order with the tool steps", async () => {
  const r = rig(new Script([call("Looking at it now."), final("All done.")]));
  const out = await r.agent.turn("fix the thing", r.opts);
  assert.deepEqual(r.log, ["say:Looking at it now.", "step:probe"]);
  assert.equal(out.segments.join(" "), "All done.");
});

test("what was said beside tool calls is kept in the history for the next turn", async () => {
  const p = new Script([call("I will check the file."), final("Found it."), final("ok")]);
  const r = rig(p);
  await r.agent.turn("look", r.opts);
  await r.agent.turn("and?", r.opts);
  const sent = JSON.stringify(p.requests[p.requests.length - 1]!.messages);
  assert.ok(sent.includes("I will check the file."));
  assert.ok(sent.includes("Found it."));
});

test("a simple turn (no tools) posts nothing and gets no nudge", async () => {
  const p = new Script([final("Hello!")]);
  const r = rig(p);
  await r.agent.turn("hi", r.opts);
  assert.deepEqual(r.log, []);
  assert.deepEqual(nudged(p), [false]);
});

test("the nudge appears after three quiet rounds, once, and stays internal", async () => {
  const p = new Script([call(""), call(""), call(""), call(""), final("end")]);
  const r = rig(p);
  const out = await r.agent.turn("long work", { ...r.opts, sandbox: true });
  assert.deepEqual(nudged(p), [false, false, false, true, false]); // none on the wrap-up call
  assert.ok(!out.segments.join(" ").includes("Internal reminder"));
  assert.ok(r.log.every((l) => !l.includes("Internal reminder")));
});

test("no nudge while the bot keeps talking", async () => {
  const p = new Script([call("Starting.", "bump"), call("Found a snag.", "bump"), call("Changed plan.", "bump"), final("end")]);
  const clock = { t: T0 };
  const r = rig(p, "pro", clock, [tool("bump", () => ((clock.t += TALK_MIN_GAP_MS + 1000), "ok"))]);
  await r.agent.turn("work", { ...r.opts, sandbox: true });
  assert.ok(!nudged(p).includes(true));
});

test("the nudge also comes after a minute of work, with fewer rounds", async () => {
  const clock = { t: T0 };
  const p = new Script([call("", "slow"), call(""), final("end")]);
  const r = rig(p, "pro", clock, [tool("slow", () => ((clock.t += 61_000), "ok"))]);
  await r.agent.turn("work", { ...r.opts, sandbox: true });
  assert.deepEqual(nudged(p), [false, true, true]);
});

test("rate cap: a second text within the gap is held back, one after the gap goes out", async () => {
  const clock = { t: T0 };
  const bump = tool("bump", () => ((clock.t += TALK_MIN_GAP_MS + 1000), "ok"));
  const p = new Script([call("First.", "bump"), call("Second."), call("Too soon.", "bump"), call("After gap."), final("end")]);
  const r = rig(p, "pro", clock, [bump]);
  await r.agent.turn("work", { ...r.opts, sandbox: true });
  assert.deepEqual(r.log.filter((l) => l.startsWith("say:")), ["say:First.", "say:Second.", "say:After gap."]);
});

test("rate cap: at most TALK_TURN_MAX texts per turn", () => {
  const t = new Talk({ mode: "owner", job: false, startedAt: 0 });
  let posted = 0;
  for (let i = 0; i < 20; i++) if (t.offer(`line ${i}`, i * 100_000) !== null) posted++;
  assert.equal(posted, TALK_TURN_MAX);
});

test("an answer to the person's own message skips the gap", () => {
  const t = new Talk({ mode: "owner", job: false, startedAt: 0 });
  assert.equal(t.offer("one", 1000), "one");
  assert.equal(t.offer("two", 2000), null);
  t.personWrote();
  assert.equal(t.offer("answer", 3000), "answer");
});

test("rooms: one short line, never a pass; off mode says nothing", () => {
  const room = new Talk({ mode: "room", job: false, startedAt: 0 });
  assert.equal(room.offer("pass", 0), null);
  assert.equal(room.offer("[PASS]", 0), null);
  const line = room.offer("x".repeat(400), 0)!;
  assert.ok(line.length <= 160);
  assert.equal(room.offer("another", 99_999), null);
  assert.equal(new Talk({ mode: "off", job: false, startedAt: 0 }).offer("hi", 0), null);
});

test("a routine run posts nothing beside tool calls", async () => {
  const r = rig(new Script([call("Chatty."), final("done")]));
  await r.agent.turn("routine", { ...r.opts, routine: true });
  assert.deepEqual(r.log.filter((l) => l.startsWith("say:")), []);
});

test("a room turn posts one short line at most", async () => {
  const r = rig(new Script([call("First line."), call("Second line."), final("done")]));
  await r.agent.turn("hey", { ...r.opts, room: true, owner: false });
  assert.deepEqual(r.log.filter((l) => l.startsWith("say:")), ["say:First line."]);
});

test("job turns talk: the stay-quiet rule is gone and the job turn gets the talk rule", async () => {
  assert.ok(!/stay quiet/.test(JOB_TURN_NOTE));
  assert.match(JOB_TURN_NOTE, /keep your owner in the loop/);
  const p = new Script([final("ok")]);
  const r = rig(p);
  await r.agent.turn("go", { ...r.opts, context: "<job_data>goal</job_data>" });
  assert.ok(JSON.stringify(p.requests[0]!.messages).includes("keep your owner in the loop"));
});

test("pro, fish and potato prompts all carry the rule", () => {
  for (const pack of ["pro", "fish", "potato"]) {
    const all = loadPersona(PERSONA_DIR, pack).map((b) => b.content).join("\n");
    assert.match(all, /Talking while you work/, pack);
  }
});

test("an ordinary turn's prompt has the code-side rule, never the internal reminder", async () => {
  const p = new Script([final("ok")]);
  const r = rig(p, "fish");
  await r.agent.turn("hi", r.opts);
  const sent = JSON.stringify(p.requests[0]!.messages);
  assert.ok(sent.includes("Talking while you work"));
  assert.ok(!sent.includes(TALK_NUDGE));
});
