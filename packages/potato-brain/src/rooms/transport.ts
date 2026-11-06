import type { ErrorCode } from "../core/errors.ts";
import type { PassReason } from "./types.ts";

export interface RoomReply {
  roomId: string;
  /** the id of the room line this answers */
  inReplyTo: string;
  segments: string[];
  kaomoji: string | null;
  tone: string | null;
}

/** The conversation id of a room on the app surface. The shape the app and the brain agree on (surface:kind:id). */
export function roomConversationId(roomId: string): string {
  return `app:room:${roomId}`;
}

/** Where a room lives. The app is the first one; a Discord channel can be a second one later. */
export interface RoomTransport {
  typing(roomId: string, on: boolean): void;
  post(reply: RoomReply): void;
  pass(roomId: string, lineId: string, why: PassReason | "no_reply"): void;
  /** the turn could not run (no key, over the limit, provider down): a code and nothing else */
  fail(roomId: string, lineId: string, code: ErrorCode): void;
}

/** The app: everything goes out as protocol events with conversationId "app:room:<id>". `emit` adds "v":1. */
export class PotatoRoomTransport implements RoomTransport {
  private readonly emit: (event: Record<string, unknown>) => void;
  constructor(emit: (event: Record<string, unknown>) => void) {
    this.emit = emit;
  }
  typing(roomId: string, on: boolean): void {
    this.emit({ type: "typing", conversationId: roomConversationId(roomId), on });
  }
  post(r: RoomReply): void {
    this.emit({
      type: "reply",
      conversationId: roomConversationId(r.roomId),
      inReplyTo: r.inReplyTo,
      segments: r.segments,
      ...(r.kaomoji ? { kaomoji: r.kaomoji } : {}),
      tone: r.tone,
    });
  }
  pass(roomId: string, lineId: string, why: PassReason | "no_reply"): void {
    this.emit({ type: "pass", conversationId: roomConversationId(roomId), inReplyTo: lineId, reason: why });
  }
  fail(roomId: string, lineId: string, code: ErrorCode): void {
    this.emit({ type: "error", code, conversationId: roomConversationId(roomId), inReplyTo: lineId });
  }
}
