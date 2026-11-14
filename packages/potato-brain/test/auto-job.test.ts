import { test } from "node:test";
import assert from "node:assert/strict";
import { Agent, promotedJobGoal, promotedToJobText } from "../src/core/loop.ts";
import type { ExtraTool } from "../src/core/loop.ts";
import { JOB_GOAL_MAX } from "../src/core/job-tools.ts";
import { initialMood } from "../src/persona/mood.ts";
import type { ChatRequest, ChatResult, Provider } from "../src/types.ts";

const usage = { promptTokens: 10, completionTokens: 5, cacheHitTokens: 0, cacheMissTokens: 10 };
const at = () => new Date(2026, 9, 29, 10, 0, 0);
const callRound = (id: string): ChatResult => ({ text: "", usage, model: "m", toolCalls: [{ id, name: "probe", arguments: "{}" }] });
const answer = (text: string): ChatResult => ({ text, usage, model: "m" });

class Scripted implements Provider {
  readonly name = "scripted";
  requests: ChatRequest[] = [];
  private steps: ChatResult[];
  constructor(steps: ChatResult[]) {
    this.steps = steps;
  }
  async chat(req: ChatRequest): Promise<ChatResult> {
    this.requests.push(req);
    return this.steps.shift() ?? answer("end");
  }
}

function rig(steps: ChatResult[], over: { jobStart?: "ok" | "refused" | "none"; owner?: boolean; context?: string } = {}) {
  const provider = new Scripted(steps);
  const jobGoals: string[] = [];
  const probe: ExtraTool = { spec: { name: "probe", description: "probe", parameters: { type: "object", properties: {} } }, run: () => "ok" };
  const tools: ExtraTool[] = [probe];
  if (over.jobStart !== "none") {
    tools.push({
      gate: "owner_person",
      spec: { name: "job_start", description: "job", parameters: { type: "object", properties: { goal: { type: "string" } } } },
      run: (args) => {
        jobGoals.push(String(args["goal"]));
        return over.jobStart === "refused" ? { text: "job_start did not run [POTATO_JOB_RUNNING]: A job is already running.", isError: true } : { text: "The job is started." };
      },
    });
  }
  const agent = new Agent({
    provider,
    persona: [{ id: "p", volatility: 0, content: "persona" }],
    mood: initialMood(at()),
    now: at,
    extraTools: tools,
    random: () => 0.5,
    log: () => {},
    sleep: async () => {},
  });
  const turn = (text: string) => agent.turn(text, { owner: over.owner ?? true, ...(over.context ? { context: over.context } : {}) });
  return { turn, jobGoals, provider };
}

const outOfSteps = (): ChatResult[] => [callRound("c1"), callRound("c2"), callRound("c3"), callRound("c4"), answer("wrap-up")];

test("a chat turn that ran out of steps becomes a job through job_start", async () => {
  const t = rig(outOfSteps());
  const res = await t.turn("build the whole site");
  assert.equal(res.segments.join(""), promotedToJobText());
  assert.match(promotedToJobText(), /Press Stop/);
  assert.equal(t.jobGoals.length, 1);
  assert.match(t.jobGoals[0]!, /build the whole site/);
  assert.equal(t.provider.requests.length, 4, "no wrap-up call: the job goes on by itself");
});

test("a final answer is not promoted", async () => {
  const t = rig([callRound("c1"), answer("all done")]);
  const res = await t.turn("quick thing");
  assert.equal(res.segments.join(""), "all done");
  assert.equal(t.jobGoals.length, 0);
});

test("a refused job_start (a job already runs) keeps the old ending", async () => {
  const t = rig(outOfSteps(), { jobStart: "refused" });
  const res = await t.turn("keep going");
  assert.equal(t.jobGoals.length, 1);
  assert.equal(res.segments.join(""), "wrap-up", "the turn ends as it did before");
});

test("no job tool, a job turn, or a non-owner turn keep the old line", async () => {
  for (const o of [{ jobStart: "none" as const }, { context: "<job_data>x</job_data>" }, { owner: false }]) {
    const t = rig(outOfSteps(), o);
    const res = await t.turn("work");
    assert.equal(t.jobGoals.length, 0);
    assert.doesNotMatch(res.segments.join(""), /as a job/);
  }
});

test("the goal is the person's words, cut to the job limit", () => {
  assert.match(promotedJobGoal("  fix   the\nbug "), /Finish the work in progress in this chat\. The person last said: fix the bug$/);
  assert.ok(promotedJobGoal("x".repeat(5000)).length <= JOB_GOAL_MAX);
});
