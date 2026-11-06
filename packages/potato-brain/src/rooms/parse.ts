import type { Member, RoomFacts, RoomLine } from "./types.ts";
import { MAX_ROOM_BOTS } from "./types.ts";
import type { RoomMeta } from "./runner.ts";

type Json = Record<string, unknown>;
const isRecord = (x: unknown): x is Json => typeof x === "object" && x !== null && !Array.isArray(x);

export const ROOM_ID = /^r[0-9a-f]{12}$/;
export const MAX_RECENT = 20;
export const MAX_LINE_TEXT = 8000;

function clean(x: unknown, max: number): string | null {
  if (typeof x !== "string") return null;
  const t = x.replace(/[\u0000-\u001f\u007f]+/g, " ").trim().slice(0, max);
  return t === "" ? null : t;
}

function member(x: unknown): Member | null {
  if (!isRecord(x)) return null;
  const kind = x["kind"];
  const id = clean(x["id"], 128);
  const name = clean(x["name"], 64);
  if ((kind !== "person" && kind !== "bot") || !id || !name) return null;
  return { kind, id, name };
}

function line(x: unknown): RoomLine | null {
  if (!isRecord(x)) return null;
  const id = clean(x["id"], 128);
  const from = member(x["from"]);
  const text = typeof x["text"] === "string" ? x["text"].slice(0, MAX_LINE_TEXT) : null;
  const at = x["at"];
  const hop = x["hop"];
  const reply = x["inReplyTo"];
  if (!id || !from || text === null || typeof at !== "number" || !Number.isFinite(at)) return null;
  if (typeof hop !== "number" || !Number.isInteger(hop) || hop < 0 || hop > 255) return null;
  const inReplyTo = typeof reply === "string" && reply !== "" ? reply.slice(0, 128) : null;
  return { id, from, text, atMs: at, hop, inReplyTo };
}

export interface ParsedRoomItem {
  /** the delivery id (for "handled once") */
  id: string;
  conversationId: string;
  facts: RoomFacts;
  meta: RoomMeta;
}

/** Validates a room inbox item (see RoomInboxItem in types.ts). Anything off returns null and the item is ignored. */
export function parseRoomItem(raw: unknown): ParsedRoomItem | null {
  if (!isRecord(raw) || raw["type"] !== "room") return null;
  const id = clean(raw["id"], 128);
  const roomId = raw["roomId"];
  if (!id || typeof roomId !== "string" || !ROOM_ID.test(roomId)) return null;
  const conversationId = `app:room:${roomId}`;
  if (raw["conversationId"] !== conversationId) return null;
  const me = member(raw["me"]);
  if (!me || me.kind !== "bot") return null;
  const rawRoster = raw["roster"];
  if (!Array.isArray(rawRoster) || rawRoster.length < 2 || rawRoster.length > MAX_ROOM_BOTS + 1) return null;
  const roster: Member[] = [];
  for (const m of rawRoster) {
    const parsed = member(m);
    if (!parsed) return null;
    roster.push(parsed);
  }
  if (!roster.some((m) => m.kind === "bot" && m.id === me.id)) return null;
  const rawRecent = raw["recent"];
  if (rawRecent !== undefined && (!Array.isArray(rawRecent) || rawRecent.length > MAX_RECENT)) return null;
  const recent: RoomLine[] = [];
  for (const l of (rawRecent ?? []) as unknown[]) {
    const parsed = line(l);
    if (!parsed) return null;
    recent.push(parsed);
  }
  const current = line(raw["line"]);
  if (!current) return null;
  const caps = raw["capabilities"];
  return {
    id,
    conversationId,
    facts: { roomId, roomName: clean(raw["roomName"], 60) ?? "Group", me, roster, recent, line: current },
    meta: {
      tools: raw["tools"] === "no_memory" ? "no_memory" : "full",
      sandbox: isRecord(caps) && caps["sandbox"] === true,
      sandboxOnly: raw["sandboxOnly"] === true,
    },
  };
}
