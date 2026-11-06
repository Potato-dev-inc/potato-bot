/**
 * Talking while working. A bot that gets a request should say, in a line or two, what it understood and what it
 * will do, then start; tell the person when something notable happens; and say at the end what is done and what
 * is left. Two parts make that real:
 *
 * - Text the model writes next to a tool call is a message to the person. The loop posts it live (in order, as a
 *   normal message) and keeps it in the history; it used to be dropped.
 * - A nudge in the code: if several tool rounds or about a minute pass with nothing said, one internal line is added
 *   to the next round's context. It costs no extra model call and never reaches the person.
 *
 * The caps live here too, so a bot cannot flood a chat. This file is pure: the loop owns the clock and the sending.
 */

/** Tool rounds without a message to the person before the nudge. */
export const TALK_QUIET_ROUNDS = 3;
/** Work time without a message to the person before the nudge. */
export const TALK_QUIET_MS = 60_000;
/** The least time between two posted mid-turn texts. The first one of a turn and answers to the person are free. */
export const TALK_MIN_GAP_MS = 8_000;
/** The same, in a job: its turns follow each other by themselves, so the gap counts across turns. */
export const TALK_JOB_MIN_GAP_MS = 45_000;
/** Mid-turn texts per turn. */
export const TALK_TURN_MAX = 6;
/** In a room: one short line, once. */
export const TALK_ROOM_TURN_MAX = 1;
export const TALK_TEXT_MAX = 500;
export const TALK_ROOM_LINE_MAX = 160;

/** The reminder added to the next round. Marked internal so the model does not repeat it to the person. */
export const TALK_NUDGE =
  "[Internal reminder, never mention it: you have been working a while without telling the person anything. If something notable happened (a finding, a change of plan, a problem), say it now in one short line before the next tool call. If not, keep working quietly.]";

/** One block for turns that can use tools: how to talk, kept the same in every persona (each persona adds its own voice). */
export const TALK_TURN_NOTE =
  "Talking while you work: for anything beyond a one-step answer, first say in one or two short lines what you understood and what you will do, then start; do not wait for a yes unless the request is truly unclear or risky. " +
  "Text you write beside a tool call reaches the person at once as a message. During longer work add a short line only when something notable happens (a finding, a change of plan, a problem), not for every step. At the end say what is done and what is left. " +
  "A greeting or a quick fact gets just the answer, with no plan. Use the person's language and keep every line brief.";

/** Same idea for a turn of a job, which the app starts by itself: replaces the old \"stay quiet\" rule. */
export const TALK_JOB_NOTE =
  "Between and inside the turns of a job, keep your owner in the loop like a teammate: when a turn starts, say in one short line what you will do next, unless it is the same as your last message. " +
  "Send a short line when something notable happens (a finding, a change of plan, a problem) and when the job is done say what is done and what is left. No line for routine steps, and never repeat yourself.";

/** A room turn: one short line at most. */
export const TALK_ROOM_NOTE = "In this group, if you do tool work first, you may say one short line before it. Never more than one.";

export type TalkMode = "owner" | "room" | "off";

export interface TalkOptions {
  mode: TalkMode;
  job: boolean;
  /** when this turn's work began, ms */
  startedAt: number;
  /** when the last mid-turn text of an earlier turn was posted (a job's gap counts across turns), ms */
  previousAt?: number;
}

/** Text that stands for "nothing to say" in a room: never posted. */
function isPass(text: string): boolean {
  const t = text.trim().toLowerCase().replace(/[\s.…。]+$/g, "");
  return t === "" || t === "pass" || t === "[pass]" || t === "no_reply" || t === "no reply";
}

/** One line, whitespace collapsed, cut with an ellipsis. */
function clampLine(text: string, max: number): string {
  const line = text.replace(/\s+/g, " ").trim();
  return line.length <= max ? line : `${line.slice(0, max - 1).trimEnd()}…`;
}

export class Talk {
  private readonly o: TalkOptions;
  private posted = 0;
  private lastPostAt: number | undefined;
  private lastSaidAt: number;
  private quietRounds = 0;
  private answering = false;
  /** every text that reached the person this turn, in order */
  readonly said: string[] = [];

  constructor(options: TalkOptions) {
    this.o = options;
    this.lastPostAt = options.previousAt;
    this.lastSaidAt = options.startedAt;
  }

  /** When the last text of this turn (or the turn before, in a job) was posted. */
  get mode(): TalkMode {
    return this.o.mode;
  }

  get lastAt(): number | undefined {
    return this.lastPostAt;
  }

  /** The person wrote something while the turn ran: the next text is an answer and skips the gap. */
  personWrote(): void {
    this.answering = true;
  }

  /** message_user (or any other message to the person) went out: the quiet stretch starts over. */
  toldPerson(now: number): void {
    this.quietRounds = 0;
    this.lastSaidAt = now;
  }

  /**
   * The text a model round wrote beside its tool calls. Returns what to post, or null (empty, a pass, over a cap).
   * A text that is not posted stays in the model's own context; it is just not shown.
   */
  offer(raw: string, now: number): string | null {
    if (this.o.mode === "off") return null;
    const room = this.o.mode === "room";
    if (room && isPass(raw)) return null;
    const text = room ? clampLine(raw, TALK_ROOM_LINE_MAX) : raw.trim().length > TALK_TEXT_MAX ? `${raw.trim().slice(0, TALK_TEXT_MAX - 1).trimEnd()}…` : raw.trim();
    if (text === "") return null;
    if (this.posted >= (room ? TALK_ROOM_TURN_MAX : TALK_TURN_MAX)) return null;
    const gap = this.o.job ? TALK_JOB_MIN_GAP_MS : TALK_MIN_GAP_MS;
    const free = this.answering || (this.posted === 0 && !this.o.job);
    if (!free && this.lastPostAt !== undefined && now - this.lastPostAt < gap) return null;
    this.answering = false;
    this.posted++;
    this.lastPostAt = now;
    this.toldPerson(now);
    this.said.push(text);
    return text;
  }

  /** A tool round finished. */
  roundDone(): void {
    this.quietRounds++;
  }

  /**
   * The internal reminder for the next round, or null. Never on a turn that is not working (no round done), never in
   * a room, and it starts a new quiet stretch so it does not repeat on every round.
   */
  nudge(now: number): string | null {
    if (this.o.mode !== "owner") return null;
    if (this.quietRounds === 0) return null;
    if (this.quietRounds < TALK_QUIET_ROUNDS && now - this.lastSaidAt < TALK_QUIET_MS) return null;
    this.quietRounds = 0;
    this.lastSaidAt = now;
    return TALK_NUDGE;
  }
}
