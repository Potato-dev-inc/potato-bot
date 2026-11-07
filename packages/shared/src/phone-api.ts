/**
 * Phone API v1 (types only, plain TypeScript, no DOM). The server is `potato/phone` in the Rust app; the wire format
 * is written down in `docs/PHONE-API.md`. The web app (`packages/app/phone`) and a later native or Capacitor app speak
 * the same JSON. Auth is the device token: `Authorization: Bearer <token>` on every call but `health`, `hello`, `pair`.
 */

export const PHONE_API_VERSION = 1;

export type PhoneCap = 'watch' | 'type' | 'answer' | 'control';

export interface PhoneError {
  error: string;
  message: string;
}

/** POST /api/v1/hello: `proof` = hex HMAC-SHA256(key, "potato-phone-hello|" + nonce). */
export interface PhoneHelloRequest {
  kind: 'pair' | 'device';
  nonce: string;
  deviceId?: string;
}
export interface PhoneHelloResponse {
  proof: string;
  pcName: string;
}

export interface PhonePairRequest {
  code: string;
  deviceName: string;
}
export interface PhonePairResponse {
  deviceId: string;
  token: string;
  pcName: string;
  candidates: string[];
}

export interface PhoneMe {
  deviceId: string;
  name: string;
  pcName: string;
  caps: PhoneCap[];
  groups: boolean;
  paused: boolean;
  candidates: string[];
}

export type PhoneBotState = 'working' | 'needs_you' | 'idle' | 'offline' | 'problem';

export interface PhoneBot {
  id: string;
  name: string;
  state: PhoneBotState;
  stateText: string;
  lastLine: string | null;
  lastAt: number | null;
  unread: number;
  openCards: number;
  typing: boolean;
}

export interface PhoneRoomSummary {
  id: string;
  name: string;
  bots: string[];
  lastMs: number;
  unread: number;
  typing: string[];
}

export interface PhoneBotList {
  rev: number;
  paused: boolean;
  bots: PhoneBot[];
  rooms: PhoneRoomSummary[];
}

export interface PhoneMessage {
  id: string;
  at: number;
  from: { kind: 'you' | 'bot' | 'person' | 'note'; name?: string };
  kind: 'text' | 'helper_card' | 'helper_result' | 'note' | 'decision_card' | 'decision_result';
  text: string;
  queued?: true;
  requestId?: string;
  file?: { id: string; name: string; kind: string; size: number };
  card?: { kind: 'approval' | 'question'; why: string; choices: string[]; keepGoing?: true; connector?: string; login?: string; answer?: string };
}

export interface PhoneCard {
  id: string;
  kind: 'approval' | 'question';
  action: string;
  why: string;
  choices: string[];
  createdMs: number;
  keepGoing: string | null;
  connector: string | null;
  login: string | null;
  mcpAdd: { name: string; url: string } | null;
  canAlwaysAllow: boolean;
}

export interface PhoneWorkStep {
  id: string;
  tool: string;
  status: 'running' | 'done' | 'error' | 'retry';
  startedMs: number;
  endedMs: number | null;
  detail?: string;
  why?: string;
}

export interface PhoneWorkRun {
  startedMs: number;
  steps: PhoneWorkStep[];
  failed: boolean;
  endedMs: number | null;
  continues?: true;
}

export interface PhoneJob {
  goal: string;
  step: string;
  startedMs: number;
  spendNano: number;
  state: string;
}

export interface PhoneBotDetail {
  rev: number;
  bot: PhoneBot;
  messages: PhoneMessage[];
  cards: PhoneCard[];
  job: PhoneJob | null;
  work: PhoneWorkRun | null;
  typing: boolean;
}

export interface PhoneSendRequest {
  text: string;
  files?: { name: string; kind: 'image'; data: string }[];
}
export interface PhoneSendResponse {
  id: string;
  delivered: boolean;
  state: string;
}

/** Card answers: approval `allow_once` | `allow_job` | `deny`; question the choice number as a string. */
export interface PhoneAnswerRequest {
  answer: string;
}

export interface PhoneChanges {
  rev: number;
}

/** WebSocket /api/v1/ws, subprotocols `pm.v1` and `pm-bearer.<token>`: text frames of these. */
export type PhoneSocketMessage = { type: 'changed'; rev: number } | { type: 'signed_out' };
