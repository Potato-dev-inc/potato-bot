import { TALK_JOB_NOTE } from "./talk.ts";
import type { ExtraTool } from "./loop.ts";
import type { SandboxReply } from "./sandbox-tools.ts";

/**
 * The job tools (Professional mode). A job is a long piece of work saved by the app as a chain of turns: when a
 * turn ends, the app starts the next one by itself with the goal, the to-do list and the step log. The brain only
 * names the job (`job_start`), notes what it finished (`job_log`) and says it is over (`job_done`); the app keeps
 * the record, the limits and the clock. Each call goes to the app as a `sandbox_call`, like the to-do tools.
 *
 * `gate: "owner_person"`: the owner's own turns, never a routine's run. The app checks the same, and also that the
 * chat is the private one. A group turn does not get them: they are in ROOM_OFF_TOOLS.
 */
export const JOB_TOOL_NAMES = ["job_start", "job_log", "job_done"] as const;

export type JobToolName = (typeof JOB_TOOL_NAMES)[number];

/** Same limits as the app (jobs.rs): a longer text is refused by the app with a plain sentence. */
export const JOB_GOAL_MAX = 600;
export const JOB_STEP_MAX = 200;
export const JOB_SUMMARY_MAX = 600;

/** One block for owner turns in the private chat: when to start a job, and how to behave while one runs. */
export const JOB_TURN_NOTE =
  "For work that will take longer than this one turn (many steps, a lot of waiting, a whole project), call job_start with the goal first: the app then starts your next turn by itself until you call job_done. Inside a job, keep working and do not ask for things you can decide yourself; log each finished step with job_log, keep your to-do list up to date, and call job_done only when the goal is really reached. " + TALK_JOB_NOTE + " When you need your owner's yes before something risky or hard to undo, call request_approval and wait for the answer; never say it was approved before the answer arrives. The app itself stops commands that push, publish, deploy, reach another computer or delete outside your work folder, and asks your owner: do not try to get around that.";

export const JOB_SPECS: Record<JobToolName, { name: string; description: string; parameters: Record<string, unknown> }> = {
  job_start: {
    name: "job_start",
    description:
      "Start a job: work that will take longer than this one turn. The app saves it and starts your next turn by itself when this one ends, again and again, until you call job_done or a limit is reached (4 hours or 5 dollars). Only one job at a time. Do not use it for a quick answer.",
    parameters: {
      type: "object",
      properties: { goal: { type: "string", description: `What the job must reach, in plain words (${JOB_GOAL_MAX} characters at most)` } },
      required: ["goal"],
    },
  },
  job_log: {
    name: "job_log",
    description:
      "Write down one step of the running job that is really finished. The app shows the log to your next turn. A turn that finishes no step and changes nothing on your to-do list counts as no progress; three in a row stop the job.",
    parameters: {
      type: "object",
      properties: { step: { type: "string", description: `One finished step in a plain line (${JOB_STEP_MAX} characters at most)` } },
      required: ["step"],
    },
  },
  job_done: {
    name: "job_done",
    description: "Say the running job is finished. The app stops starting turns for it. Then tell your owner the result in your reply.",
    parameters: {
      type: "object",
      properties: { summary: { type: "string", description: `The result in a line or two (${JOB_SUMMARY_MAX} characters at most)` } },
      required: ["summary"],
    },
  },
};

/** What the model reads from a job call: a plain line, never the app's internals. */
export function formatJobReply(tool: string, reply: SandboxReply): { text: string; isError?: true } {
  if (!reply.ok) {
    const { code, message } = reply.error;
    return { text: `${tool} did not run [${code}]: ${message}`, isError: true };
  }
  const note = typeof reply.result["note"] === "string" ? ` ${reply.result["note"]}` : "";
  const word = tool === "job_start" ? "The job is started." : tool === "job_log" ? "Step logged." : "The job is marked done.";
  return { text: `${word}${note}` };
}

/** The three tools for one conversation. */
export function jobTools(opts: {
  call: (tool: JobToolName, args: Record<string, unknown>) => Promise<SandboxReply>;
}): ExtraTool[] {
  return JOB_TOOL_NAMES.map((name): ExtraTool => ({
    spec: { ...JOB_SPECS[name] },
    gate: "owner_person",
    async run(args) {
      let reply: SandboxReply;
      try {
        reply = await opts.call(name, args);
      } catch {
        reply = { ok: false, error: { code: "JOB_FAILED", message: "The job record did not answer." } };
      }
      return formatJobReply(name, reply);
    },
  }));
}
