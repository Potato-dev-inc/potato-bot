import { codeOf } from "../core/errors.ts";
import { makeGate, type GateMode } from "./gate.ts";
import { roomContext } from "./prompt.ts";
import { SilenceBudget } from "./silence.ts";
import { roomConversationId, type RoomTransport } from "./transport.ts";
import type { RoomFacts, Verdict } from "./types.ts";

export interface TurnOut { segments: string[]; kaomoji: string | null; tone: string | null }

/** What the app said about this delivery, besides the line itself. */
export interface RoomMeta {
  tools: "full" | "no_memory";
  /** the app said this bot has its computer on this turn */
  sandbox: boolean;
  /** the author is not the owner (a stranger); version 1 never sets it */
  sandboxOnly: boolean;
}

/** One room turn, ready for the channel to run on this bot's agent. */
export interface RoomTurnInput {
  conversationId: string;
  /** the id of the room line being answered */
  replyTo: string;
  /** the line, as the user message: "[Name] text" */
  userText: string;
  /** TurnOptions.context: user text, fenced */
  context: string;
  /** true only for a line the person (the owner) wrote; a bot's line never gets owner tools */
  owner: boolean;
  sandbox: boolean;
  tools: "full" | "no_memory";
}

export interface RunnerOptions {
  transport: RoomTransport;
  /** Runs one normal turn on the agent of `conversationId`. null = nothing to say. May throw: the runner reports the code. */
  turn: (t: RoomTurnInput) => Promise<TurnOut | null>;
  now: () => number;
  mode?: GateMode;
}

const TEXT_CHARS = 4000;

export class RoomRunner {
  private readonly gate;
  private readonly silence = new SilenceBudget();
  private readonly opts: RunnerOptions;
  constructor(opts: RunnerOptions) {
    this.opts = opts;
    this.gate = makeGate(opts.mode ?? "heuristic");
  }

  /** Decide, then maybe run a turn. Exactly one of post, pass or fail reaches the transport per call. */
  async onLine(f: RoomFacts, meta: RoomMeta): Promise<Verdict> {
    const t = this.opts.transport;
    const now = this.opts.now();
    const v = this.gate(f, { silence: this.silence, now });
    if (v.kind === "pass") {
      t.pass(f.roomId, f.line.id, v.why);
      return v;
    }
    t.typing(f.roomId, true);
    try {
      const out = await this.opts.turn({
        conversationId: roomConversationId(f.roomId),
        replyTo: f.line.id,
        userText: `[${f.line.from.name}] ${f.line.text.slice(0, TEXT_CHARS)}`,
        context: roomContext(f),
        owner: f.line.from.kind === "person" && !meta.sandboxOnly,
        sandbox: meta.sandbox,
        tools: meta.tools,
      });
      if (!out || out.segments.length === 0) {
        t.pass(f.roomId, f.line.id, "no_reply");
        return v;
      }
      if (v.why === "question" || v.why === "chatty") this.silence.spend(now);
      t.post({ roomId: f.roomId, inReplyTo: f.line.id, ...out });
    } catch (err) {
      t.fail(f.roomId, f.line.id, codeOf(err));
    } finally {
      t.typing(f.roomId, false);
    }
    return v;
  }
}
