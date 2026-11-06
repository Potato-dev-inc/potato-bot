/**
 * Error codes of channel protocol v1. An error that leaves the brain carries
 * one of these and nothing else: never a response body, a header or a key.
 */
export const ERROR_CODES = [
  "PROVIDER_AUTH",
  "PROVIDER_LIMIT",
  "PROVIDER_DOWN",
  "NETWORK",
  "PERSONA_TOO_BIG",
  "SURFACE_AUTH",
  "INTERNAL",
] as const;

export type ErrorCode = (typeof ERROR_CODES)[number];

export class CodedError extends Error {
  readonly code: ErrorCode;
  readonly status: number | undefined;
  /** A short fixed-form reason for the log ("http 503", "empty reply finish=length"). Never body text, url, headers or keys. */
  readonly reason: string | undefined;

  constructor(code: ErrorCode, message: string, status?: number, reason?: string) {
    super(message);
    this.reason = reason;
    this.name = "CodedError";
    this.code = code;
    this.status = status;
  }
}

/** Map an HTTP status of a model endpoint to a code. The body is never read. */
export function codeForStatus(status: number): ErrorCode {
  if (status === 401 || status === 403) return "PROVIDER_AUTH";
  // 402 is "out of balance" on the endpoints we talk to, 429 is a rate limit.
  if (status === 402 || status === 429) return "PROVIDER_LIMIT";
  if (status >= 500) return "PROVIDER_DOWN";
  return "INTERNAL";
}

export function isCodedError(err: unknown): err is CodedError {
  return err instanceof CodedError;
}

/** Anything thrown becomes one code. Unknown errors are INTERNAL. */
export function codeOf(err: unknown): ErrorCode {
  return isCodedError(err) ? err.code : "INTERNAL";
}

/** The safe reason of a failure, or "" when it has none. Only the fixed forms set by the provider reach it. */
export function reasonOf(err: unknown): string {
  if (!isCodedError(err) || !err.reason) return "";
  return err.reason.replace(/[^\w =.:-]/g, "").slice(0, 60);
}

/** What a turn had done when it died: kept beside the error, never inside its text. */
export interface TurnFailure {
  /** tool calls that ran before the failure */
  steps: number;
  /** names (registry names only) of the last tools that ran, oldest first */
  lastTools: string[];
  /** the model usage already spent by finished rounds, so a failed turn is still counted */
  usage: { promptTokens: number; completionTokens: number; cacheHitTokens: number; cacheMissTokens: number };
  model: string;
  /** which round failed (1-based) and how big the request was: numbers only, no text */
  round: number;
  messages: number;
  approxPromptTokens: number;
}

const FAILURES = new WeakMap<object, TurnFailure>();

export function noteTurnFailure(err: unknown, failure: TurnFailure): void {
  if (typeof err === "object" && err !== null) FAILURES.set(err, failure);
}

export function turnFailureOf(err: unknown): TurnFailure | undefined {
  return typeof err === "object" && err !== null ? FAILURES.get(err) : undefined;
}

/** The numbers-only log suffix of a failed turn. */
export function failureDetail(f: TurnFailure): string {
  return `round=${f.round} messages=${f.messages} approx_prompt_tokens=${f.approxPromptTokens} steps=${f.steps}`;
}
