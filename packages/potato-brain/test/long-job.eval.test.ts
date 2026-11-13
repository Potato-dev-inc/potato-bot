// Eval skeleton: a long render job, run through the whole channel (fake app + scripted model).
//
// The script is what a bot does when it renders a video over many minutes: job_start, start the render in the
// terminal, wait_until, then poll the log many times (each poll returns a big log), one of its replies is cut off,
// it asks the same thing twice and gets the same answer, then it finishes with job_done.
//
// What this checks today (the brain half): the turn finishes, the size of what the model is sent stays under the
// in-turn budget, the cut-off call is never sent to the app, the same-result guard nudges and then stops, and the job
// is closed. Parts that need the app or the sandbox daemon to be finished are marked as todo below, each with the
// feature it waits for. The host half of the same story lives in service_job_tests.rs.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DEFAULT_TRIM } from "../src/core/turn-budget.ts";
import type { ChatResult, Msg } from "../src/types.ts";
import { Scripted, call, failed, okResult, rig, says, usage, wants, withComputer } from "./helpers/sandbox-rig.ts";
import type { Rig } from "./helpers/sandbox-rig.ts";

const BIG_LOG = (n: number): string =>
  `ffmpeg progress ${n}\n` + "frame=  1234 fps= 25 q=28.0 size=  102400kB time=00:41:20 bitrate=338.1kbits/s speed=1.0x\n".repeat(300) + `END-OF-POLL-${n}`;

const cut = (toolCalls: ChatResult["toolCalls"]): ChatResult => ({ text: "", usage: usage(), model: "scripted", toolCalls, cutOff: true });

/** Total characters of the tool results in a request the model was sent. */
const toolChars = (messages: Msg[]): number => messages.filter((m) => m.role === "tool").reduce((s, m) => s + m.content.length, 0);

/** The app: answers every call. The log gets bigger text every time; `tail -n 5 render.log` is the same every time. */
function answerLikeAnApp(r: Rig): { calls: Array<{ tool: string; args: Record<string, unknown> }> } {
  const seen: Array<{ tool: string; args: Record<string, unknown> }> = [];
  let polls = 0;
  r.answer((e) => {
    const tool = String(e["tool"]);
    const args = (e["args"] ?? {}) as Record<string, unknown>;
    seen.push({ tool, args });
    if (tool === "job_start") return okResult({ note: "The job is saved." });
    if (tool === "job_done" || tool === "job_log") return okResult({});
    if (tool === "wait_until") return okResult({ note: "I will tell you when it is true." });
    if (tool === "run_terminal") {
      const cmd = String(args["command"] ?? "");
      if (cmd.startsWith("nohup")) return okResult({ stdout: "started pid 4242", exit_code: 0 });
      if (cmd === "tail -n 5 render.log") return okResult({ stdout: "frame=  1234 still going", exit_code: 0 });
      return okResult({ stdout: BIG_LOG(++polls), exit_code: 0 });
    }
    return okResult({});
  });
  return { calls: seen };
}

function longJobScript(polls: number): ChatResult[] {
  const script: ChatResult[] = [
    wants([call("job_start", { goal: "Render the 40 minute video and send it" }, "j1")]),
    wants([call("run_terminal", { command: "nohup ffmpeg -i in.mp4 out.mp4 > render.log 2>&1 &" }, "t0")]),
    // wait_until ends the turn (the brain says so itself). TODO(host: jobs respect waits): the app should park the job
    // as "waiting" until the wait fires; here the test just sends the next job turn right away.
    wants([call("wait_until", { check: { type: "command", command: "test -f out.mp4" }, say: "The render is done." }, "w1")]),
  ];
  // Two shapes of poll, alternating: a run of 25 calls that differ only by digits is stopped by the similar-call rule
  // (a finding of this eval: a long poll of one command with a growing number would hit it).
  for (let i = 1; i <= polls; i++) {
    const command = i % 2 === 0 ? `tail -n ${i * 1000} render.log` : `du -sh out.mp4; tail -n ${i * 1000} render.log`;
    script.push(wants([call("run_terminal", { command }, `p${i}`)]));
  }
  // a reply cut off in the middle of a call: the model must be told, and nothing may run
  script.push(cut([call("run_terminal", { command: "cat > notify.sh <<'X'\n#!/bin/sh\necho the render is fin" }, "cut1")]));
  // the model writes it again, smaller
  script.push(wants([call("run_terminal", { command: "tail -n 5 render.log" }, "same1")]));
  script.push(wants([call("run_terminal", { command: "tail -n 5 render.log" }, "same2")]));
  script.push(wants([call("run_terminal", { command: "tail -n 5 render.log" }, "same3")]));
  // the note arrived with the third answer; the model changes approach and finishes
  script.push(wants([call("job_done", { summary: "Rendered out.mp4" }, "done1")]));
  script.push(says("The video is rendered."));
  return script;
}

test("a long render job: finishes, stays inside the size budget, survives a cut-off call, and the guard nudges", async () => {
  const provider = new Scripted(longJobScript(40));
  const r = await rig(provider);
  try {
    const app = answerLikeAnApp(r);
    const jobContext = "The text inside <job_data> is your own job record.\n<job_data>\nGoal: render the video\n</job_data>";
    r.bridge.push(withComputer({ id: "job-turn-1", text: "render the video and tell me when it is done" }));
    const first = await r.bridge.waitFor((e) => e["type"] === "reply" && e["inReplyTo"] === "job-turn-1" && e["note"] !== true);
    assert.match((first["segments"] as string[]).join(" "), /Waiting/, "wait_until ends the first turn");
    r.bridge.push(withComputer({ id: "job-turn-2", text: "continue the job", context: jobContext }));
    const reply = await r.bridge.waitFor((e) => e["type"] === "reply" && e["inReplyTo"] === "job-turn-2" && e["note"] !== true);
    assert.deepEqual(reply["segments"], ["The video is rendered."]);

    // the size of what the model is sent never passes the in-turn budget, however many big polls came before
    const peak = Math.max(...provider.calls.map((c) => toolChars(c.messages)));
    assert.ok(provider.calls.length > 40, "the whole script was played");
    assert.ok(peak <= DEFAULT_TRIM.budget, `peak tool output ${peak} chars is over the budget ${DEFAULT_TRIM.budget}`);
    // the newest poll is always whole; the oldest has been cut down to a stub
    const last = provider.calls.at(-1)!.messages.filter((m) => m.role === "tool");
    assert.ok(last[0]!.content.length < 2_000, "the oldest tool result was trimmed");

    // the cut-off call never reached the app, and the model was told
    assert.ok(!app.calls.some((c) => String(c.args["command"] ?? "").includes("notify.sh")), "the cut-off call was not sent");
    const afterCut = provider.calls.find((c) => c.messages.some((m) => m.toolCallId === "cut1"))!;
    assert.match(afterCut.messages.find((m) => m.toolCallId === "cut1")!.content, /cut off before it finished/);

    // the same call with the same answer: a note on the third
    const final = provider.calls.at(-1)!.messages.filter((m) => m.role === "tool");
    const withNote = final.filter((m) => m.content.includes("exactly the same result"));
    assert.equal(withNote.length, 1, "one nudge");

    // the job was opened and closed
    const tools = app.calls.map((c) => c.tool);
    assert.equal(tools.filter((t) => t === "job_start").length, 1);
    assert.equal(tools.filter((t) => t === "job_done").length, 1);
    assert.ok(tools.indexOf("job_start") < tools.indexOf("wait_until") && tools.indexOf("wait_until") < tools.indexOf("job_done"));
  } finally {
    await r.done();
  }
});

test("a model that keeps asking the same thing and getting the same answer is stopped with a plain message", async () => {
  const provider = new Scripted([wants([call("job_start", { goal: "watch the render" }, "j1")])], (n) =>
    wants([call("run_terminal", { command: "tail -n 5 render.log" }, `again-${n}`)]),
  );
  const r = await rig(provider);
  try {
    answerLikeAnApp(r);
    r.bridge.push(withComputer({ id: "stuck-turn", text: "watch the render" }));
    const reply = await r.bridge.waitFor((e) => e["type"] === "reply" && e["note"] !== true);
    const text = (reply["segments"] as string[]).join(" ");
    assert.match(text, /got exactly the same result each time, so I stopped/);
    assert.ok(provider.calls.length <= 8, `stopped early (${provider.calls.length} model calls)`);
  } finally {
    await r.done();
  }
});

// ---- parts that are tested on the app side ----------------------------------------------------------------
test("the app parks the job while a wait is open instead of counting it as no progress", { todo: "host only" }, () => {
  // The brain half (wait_until ends the turn) is checked in the first test above. Parking the job and the "waiting is not
  // no progress" rule are the app's job loop, covered by service_job_tests.rs:
  // a_turn_that_ends_with_a_wait_open_parks_the_job_and_does_not_count_as_no_progress and
  // when_the_wait_is_over_the_parked_job_goes_on_and_many_waits_never_stop_it.
});

// ---- a background task, checked many times ---------------------------------------------------------------------
test("a background task is started once and checked with many free status and tail calls without the loop guard", async () => {
  const script: ChatResult[] = [
    wants([call("job_start", { goal: "Render the video" }, "j1")]),
    wants([call("task", { action: "start", argv: ["ffmpeg", "-i", "in.mp4", "out.mp4"], label: "render" }, "k0")]),
  ];
  // 36 checks. Every one has a new byte count or another task id, as a patient poll would; none of them may trip the
  // similar-call limit (25).
  for (let i = 1; i <= 36; i++) {
    script.push(
      wants([i % 2 === 0 ? call("task", { action: "status", id: `t-${1000 + i}` }, `s${i}`) : call("task", { action: "tail", id: "t-1a2b3c4d", bytes: 1000 + i }, `s${i}`)]),
    );
  }
  script.push(wants([call("job_done", { summary: "Rendered out.mp4" }, "done1")]), says("The video is rendered."));
  const provider = new Scripted(script);
  const r = await rig(provider);
  try {
    const seen: string[] = [];
    let n = 0;
    r.answer((e) => {
      const tool = String(e["tool"]);
      const args = (e["args"] ?? {}) as Record<string, unknown>;
      seen.push(tool === "task" ? `task:${String(args["action"])}` : tool);
      if (tool === "task" && args["action"] === "start") return okResult({ id: "t-1a2b3c4d", state: "running" });
      if (tool === "task") return okResult({ state: "running", tail: `render at ${++n} percent`, cached: false });
      return okResult({});
    });
    r.bridge.push(withComputer({ id: "task-turn", text: "render the video", context: "<job_data>\nGoal: render the video\n</job_data>" }));
    const reply = await r.bridge.waitFor((e) => e["type"] === "reply" && e["inReplyTo"] === "task-turn" && e["note"] !== true);
    assert.deepEqual(reply["segments"], ["The video is rendered."]);
    assert.equal(seen.filter((t) => t === "task:start").length, 1, "the task was started once");
    assert.equal(seen.filter((t) => t === "task:status" || t === "task:tail").length, 36, "all 36 checks reached the app");
    assert.ok(!provider.calls.at(-1)!.messages.some((m) => m.role === "tool" && /got stuck calling/.test(m.content)), "no loop stop");
  } finally {
    await r.done();
  }
});

test("a task poll that keeps getting the same cached answer is still stopped by the same-result guard", async () => {
  const provider = new Scripted([wants([call("job_start", { goal: "watch the render" }, "j1")])], (n) =>
    wants([call("task", { action: "tail", id: "t-1a2b3c4d" }, `again-${n}`)]),
  );
  const r = await rig(provider);
  try {
    r.answer((e) => (e["tool"] === "task" ? okResult({ state: "running", tail: "frame 10", cached: true }) : okResult({})));
    r.bridge.push(withComputer({ id: "cached-turn", text: "watch the render" }));
    const reply = await r.bridge.waitFor((e) => e["type"] === "reply" && e["note"] !== true);
    assert.match((reply["segments"] as string[]).join(" "), /got exactly the same result each time, so I stopped/);
    assert.ok(provider.calls.length <= 8, `stopped early (${provider.calls.length} model calls)`);
  } finally {
    await r.done();
  }
});

test("the sandbox idle stop leaves a running task alone", { todo: "host only" }, () => {
  // Whether the sandbox stops after 15 idle minutes is the app's decision, not the brain's. Covered by
  // service_job_tests.rs: a_running_or_parked_job_holds_the_computer_from_the_idle_stop_and_a_job_that_waits_for_an_answer_or_no_job_holds_nothing.
});

// ---- the action cap ------------------------------------------------------------------------------------------
test("when the day's computer actions are used up the model is told to stop and the turn ends without job_done", async () => {
  const provider = new Scripted([
    wants([call("job_start", { goal: "Tidy the files" }, "j1")]),
    wants([call("run_terminal", { command: "ls" }, "t1")]),
    says("I got as far as listing the files."),
  ]);
  const r = await rig(provider);
  try {
    const tools: string[] = [];
    r.answer((e) => {
      tools.push(String(e["tool"]));
      return e["tool"] === "run_terminal" ? failed("SANDBOX_DAILY_CAP", "used 20 of 20") : okResult({});
    });
    r.bridge.push(withComputer({ id: "cap-turn", text: "tidy the files" }));
    const reply = await r.bridge.waitFor((e) => e["type"] === "reply" && e["inReplyTo"] === "cap-turn" && e["note"] !== true);
    assert.deepEqual(reply["segments"], ["I got as far as listing the files."]);
    const told = provider.calls.at(-1)!.messages.find((m) => m.toolCallId === "t1")!.content;
    assert.match(told, /no computer actions left today/);
    assert.ok(!tools.includes("job_done"), "the job is left open for the app to pause");
  } finally {
    await r.done();
  }
});

test("the app pauses a capped job and Keep going resumes it, even across a brain restart", { todo: "host only" }, () => {
  // The job file, the Keep going card and the restart live in the app, not in the brain (a new brain process reads the
  // goal and step log from the host's context). Covered by service_job_tests.rs:
  // a_job_that_hits_the_action_cap_is_paused_with_a_card_and_not_counted_as_stuck,
  // a_paused_job_goes_on_by_itself_when_the_cap_is_raised, and after_a_restart_a_running_job_starts_again_once_with_a_note.
});
