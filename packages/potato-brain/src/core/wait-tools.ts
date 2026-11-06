import type { ExtraTool } from "./loop.ts";
import type { SandboxReply } from "./sandbox-tools.ts";

/**
 * wait_until: "tell me when this is true". The bot gives a CHECK (a read-only command for its computer, or a web address),
 * how often to look and how long to keep looking. The app keeps the wait, looks on a timer with no model, and starts ONE
 * new turn with the result when the check passes, the time runs out, or the check is no longer allowed. This turn ends
 * here, with the app's own sentence as its reply, so nothing is spent while waiting. Written for Puppet Master from the idea
 * of OpenMausBot's waits; no code copied.
 *
 * wait_cancel: gives up one of the bot's open waits.
 *
 * `gate: "owner_person"`: the owner's own turns only, never a routine's run or a woken turn (a wait cannot start another
 * wait by itself). The app checks the chat (not a group), the trust, the cap per bot, the switches and the read-only check again.
 */
export const WAIT_TOOL_NAMES = ["wait_until", "wait_cancel"] as const;

export type WaitToolName = (typeof WAIT_TOOL_NAMES)[number];

const NOTE_CHARS = 120;

export const WAIT_SPECS: Record<WaitToolName, { name: string; description: string; parameters: Record<string, unknown> }> = {
  wait_until: {
    name: "wait_until",
    description:
      "Wait until something is true, then come back. Use it when the thing you need will take a while (a build, a download, a page going live) and you would otherwise sit and poll. Give a check: a read-only command for your computer (only looking: ls, cat, grep, stat, test, git status ...; it counts as true when it exits 0, so test -f file or grep -q word file) or a web address (true when the page loads, and holds the words in contains when you give them). The app looks every so often with no cost and starts a new turn for you with the result when it is true or the time runs out. This ends your turn: your reply is the app's one line saying you are waiting. Only in the owner's own chat; at most 5 open waits.",
    parameters: {
      type: "object",
      properties: {
        check: { type: "string", description: "A read-only command that exits 0 when the thing is true, or a web address (http or https). One line, 1000 characters at most" },
        every: { type: "string", description: 'How often to look: "90s", "5m", "2h" (60 seconds to 24 hours). Default 5m' },
        timeout: { type: "string", description: 'How long to keep looking: "10m", "2h", "1d" (up to 7 days). Default 1h' },
        note: { type: "string", description: `What you are waiting for, in a few words (${NOTE_CHARS} characters at most)` },
        contains: { type: "string", description: "Web address only: words the page must also hold (without regard to case)" },
      },
      required: ["check"],
    },
  },
  wait_cancel: {
    name: "wait_cancel",
    description: "Give up one of your open waits. Use the id wait_until gave you.",
    parameters: { type: "object", properties: { id: { type: "string", description: "The wait's id" } }, required: ["id"] },
  },
};

/** What the model reads from a wait call. An error is a plain sentence with its code. */
export function formatWaitReply(tool: string, reply: SandboxReply): { text: string; isError?: true; endTurn?: true } {
  if (!reply.ok) {
    const { code, message } = reply.error;
    return { text: `${tool} did not run [${code}]: ${message}`, isError: true };
  }
  if (tool === "wait_until") {
    const message = typeof reply.result["message"] === "string" ? reply.result["message"] : "Waiting. I will come back when it is done.";
    // The turn ends with the app's sentence as the reply: no further model round, no cost while waiting.
    return { text: message, endTurn: true };
  }
  return { text: "That wait is cancelled." };
}

export function waitTools(opts: {
  call: (tool: WaitToolName, args: Record<string, unknown>) => Promise<SandboxReply>;
}): ExtraTool[] {
  return WAIT_TOOL_NAMES.map((name): ExtraTool => ({
    spec: { ...WAIT_SPECS[name] },
    gate: "owner_person",
    async run(args) {
      let reply: SandboxReply;
      try {
        reply = await opts.call(name, args);
      } catch {
        reply = { ok: false, error: { code: "WAIT_FAILED", message: "The app did not answer." } };
      }
      return formatWaitReply(name, reply);
    },
  }));
}
