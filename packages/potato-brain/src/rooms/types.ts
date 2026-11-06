export const MAX_ROOM_BOTS = 10;
/** A person line is hop 0. Bots may answer bots up to hop 3, then they stop. */
export const MAX_BOT_HOPS = 3;

/** "listen": a line to nobody is answered by one bot. "mentions": a bot speaks only when called with @, by name, or in a reply to it. */
export type RoomMode = "listen" | "mentions";

export interface Member { kind: "person" | "bot"; id: string; name: string }

export interface RoomLine {
  id: string;
  from: Member;
  text: string;
  atMs: number;
  hop: number;
  inReplyTo: string | null;
}

/** What one bot's brain gets for one new line. */
export interface RoomFacts {
  roomId: string;
  roomName: string;
  me: Member;            // this bot
  roster: Member[];      // everyone in the room, the person first
  recent: RoomLine[];    // up to 20 lines before `line`, oldest first
  line: RoomLine;        // the new line
  /** Group settings (from the app). Absent in older items: listen, no instructions, no folder. */
  mode?: RoomMode;
  instructions?: string; // what the owner wrote for this group, at most 2000 chars
  folder?: string;       // the shared folder inside the bot's computer
}

export type PassReason =
  | "own_line" | "hop_limit" | "aimed_elsewhere" | "bot_not_addressed"
  | "left_alone" | "silence_budget" | "cooldown" | "not_first_responder" | "low_score" | "mentions_only";

export type Verdict =
  | { kind: "speak"; why: "addressed" | "everyone" | "question" | "chatty" }
  | { kind: "pass"; why: PassReason };

// ---------------------------------------------------------------------------------------------------------------
// Wire shapes between the app (Rust, G7) and this brain. Every item and event also carries "v":1, and an unknown
// "type" is ignored both ways. Field names are camelCase and identical to the Rust `RoomLine`/`RoomFrom` serde output.
// ---------------------------------------------------------------------------------------------------------------

/** A room line as the app stores and sends it. `at` is epoch milliseconds (the brain's RoomLine calls it `atMs`). */
export interface WireRoomLine {
  id: string;                 // "l" + 12 hex
  from: Member;               // { kind: "person" | "bot", id, name }; the person is { kind: "person", id: "owner", name }
  text: string;               // at most 8000 chars
  at: number;                 // epoch ms
  hop: number;                // 0 for a person's line; a bot's reply = hop of the line it answers + 1
  inReplyTo: string | null;   // id of the line this one answers, or null
}

/**
 * Inbox item: one new room line, for ONE bot's brain. The key is `type`, not `kind`.
 *   v               always 1
 *   type            always "room"
 *   id              unique id of this delivery; a repeated id is handled once
 *   roomId          "r" + 12 lowercase hex
 *   roomName        the group's name, at most 60 chars
 *   conversationId  always "app:room:<roomId>" (the brain refuses any other value)
 *   me              this bot: { kind: "bot", id, name }; its id is in `roster`
 *   roster          everyone in the room, the person first; 2..6 members (the person and 1..5 bots)
 *   recent          up to 20 lines before `line`, oldest first
 *   line            the new line
 *   tools           "full" (default) or "no_memory", as for `send`
 *   capabilities    { sandbox: true } when this bot has its computer on this turn, as for `send`
 *   sandboxOnly     true when the author is not the owner: the turn is then a stranger's (no owner tools).
 *                   Version 1 sends false or leaves it out.
 */
export interface RoomInboxItem {
  v: 1;
  type: "room";
  id: string;
  roomId: string;
  roomName: string;
  conversationId: string;
  me: Member;
  roster: Member[];
  recent: WireRoomLine[];
  line: WireRoomLine;
  tools?: "full" | "no_memory";
  capabilities?: { sandbox?: boolean };
  sandboxOnly?: boolean;
  mode?: RoomMode;            // "listen" (default) or "mentions"
  instructions?: string;      // the owner's instructions for this group
  folder?: string;            // the group's shared folder, a path inside the bot's computer
}

/**
 * Event: the bot decided not to speak on a line (or its turn gave nothing to say). The app saves nothing.
 *   type "pass", conversationId "app:room:<roomId>", inReplyTo = the `line.id` it looked at,
 *   reason = a PassReason, or "no_reply" when the turn was empty, or "failed" when the turn threw.
 */
export interface RoomPassEvent {
  type: "pass";
  conversationId: string;
  inReplyTo: string;
  reason: PassReason | "no_reply" | "failed";
}

/**
 * Events a room turn also sends, with the same fields `send` turns use:
 *   { type: "typing", conversationId, on: true | false }
 *   { type: "reply", conversationId, inReplyTo: <line.id>, segments: string[1..6], kaomoji?: string, tone }
 *   { type: "mood", ... } and { type: "usage", ... }
 * A room turn sends exactly one of reply or pass per delivered item, except when the bot is paused or has no key:
 * then it sends { type: "error", code, conversationId, inReplyTo } instead.
 */
export type RoomTypingEvent = { type: "typing"; conversationId: string; on: boolean };
