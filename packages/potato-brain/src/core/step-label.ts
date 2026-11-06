/**
 * What a step did, in plain words, for the step list: "Ran: ffmpeg -i clip.mp4 out.mp4", "Wrote report.md (2.1 kB)",
 * "Read notes.txt", "Opened example.com", "Clicked at 412, 300", "Searched memory for "invoice"".
 * Built from the call's arguments and result, so every piece of text goes through `scrubText` first: known secrets,
 * token shapes, `NAME=value` assignments, passwords and keys on a command line, and the credentials in a URL never
 * leave this file. The app scrubs once more before it shows or saves anything.
 */
import { redactSecrets } from "../lifted/redact-secrets.ts";

export const DETAIL_MAX = 120;
export const FULL_MAX = 600;
export const TAIL_LINES = 10;
export const TAIL_MAX = 800;
export const WHY_MAX = 120;
const ERROR_MAX = 160;

const TOKEN_SHAPES: RegExp[] = [
  /\bsk-[A-Za-z0-9_-]{6,}/g,
  /\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{6,}/g,
  /github_pat_[A-Za-z0-9_]{6,}/g,
  /glpat-[A-Za-z0-9_-]{10,}/g,
  /\b(?:AKIA|ASIA)[0-9A-Z]{12,}/g,
  /\bAIza[0-9A-Za-z_-]{30,}/g,
  /\b(?:sk|pk|rk)_(?:live|test)_[0-9A-Za-z]{10,}/g,
  /\bxox[abprs]-[A-Za-z0-9-]{6,}/g,
  /\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{4,}\.[A-Za-z0-9_-]*/g,
  /\bbearer\s+[A-Za-z0-9._~+/=-]{8,}/gi,
  /\bauthorization\s*[:=]\s*(?:basic|token|digest|bearer)?\s*\S{6,}/gi,
  /\b[a-z][a-z0-9+.-]*:\/\/[^\s/:@]+:[^\s/@]+@/gi,
  /-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY-----|$)/g,
];
const SECRET_NAME = /(token|secret|pass|pwd|key|auth|cred|cookie|session|bearer)/i;

/** Secrets out of one piece of text. Never throws. */
export function scrubText(text: string, secrets: readonly string[] = []): string {
  let out = redactSecrets(text, secrets);
  for (const re of TOKEN_SHAPES) out = out.replace(re, "[redacted]");
  // NAME=value: any shell-style environment value (upper-case or secret-looking name) is hidden, the name stays.
  out = out.replace(/\b([A-Za-z_][A-Za-z0-9_]*)=("[^"]*"|'[^']*'|\S+)/g, (whole, name: string) =>
    SECRET_NAME.test(name) || (name.length >= 3 && name === name.toUpperCase()) ? `${name}=[redacted]` : whole,
  );
  // --password x, --token x, -p x style flags with their value
  out = out.replace(/(--?(?:password|passwd|pass|token|secret|api[-_]?key|apikey|auth|key)(?:=|\s+))("[^"]*"|'[^']*'|\S+)/gi, "$1[redacted]");
  // a long unbroken run is a key, not a word
  out = out.replace(/[A-Za-z0-9+/_=-]{40,}/g, "[redacted]");
  return out;
}

function oneLine(text: string, max: number): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length <= max ? flat : `${flat.slice(0, Math.max(0, max - 1))}…`;
}

function baseName(path: string): string {
  const parts = path.replace(/["'`]/g, "").split(/[\\/]/).filter((p) => p !== "");
  return parts.length === 0 ? path : (parts[parts.length - 1] as string);
}

function hostOf(url: string): string {
  try {
    return new URL(url).hostname;
  } catch {
    return "";
  }
}

function size(bytes: number): string {
  if (bytes < 1000) return `${bytes} B`;
  if (bytes < 1_000_000) return `${(bytes / 1000).toFixed(1)} kB`;
  return `${(bytes / 1_000_000).toFixed(1)} MB`;
}

const str = (x: unknown): string => (typeof x === "string" ? x : "");

export interface StepFacts {
  /** one line, up to 120 characters */
  detail?: string;
  /** the full command, path or address, up to 600 characters (may span lines) */
  full?: string;
  /** the model's own short reason for the call, when it gave one */
  why?: string;
}

/** The step's plain description. Anything unknown gets nothing, and the tool's own words are shown. */
export function stepFacts(tool: string, args: Record<string, unknown>, secrets: readonly string[] = []): StepFacts {
  const clean = (text: string, max: number): string => oneLine(scrubText(text, secrets), max);
  let detail = "";
  let full = "";
  const path = str(args["path"]);
  switch (tool) {
    case "run_terminal": {
      const command = str(args["command"]);
      if (command.trim() === "") break;
      detail = `Ran: ${clean(command, DETAIL_MAX - 5)}`;
      full = scrubText(command, secrets).slice(0, FULL_MAX);
      break;
    }
    case "read_file":
      if (path) (detail = `Read ${clean(baseName(path), 80)}`), (full = scrubText(path, secrets).slice(0, FULL_MAX));
      break;
    case "write_file":
      if (path) {
        const bytes = typeof args["content"] === "string" ? Buffer.byteLength(args["content"]) : -1;
        detail = `Wrote ${clean(baseName(path), 80)}${bytes >= 0 ? ` (${size(bytes)})` : ""}`;
        full = scrubText(path, secrets).slice(0, FULL_MAX);
      }
      break;
    case "edit_file":
      if (path) (detail = `Edited ${clean(baseName(path), 80)}`), (full = scrubText(path, secrets).slice(0, FULL_MAX));
      break;
    case "list_files":
      if (path) (detail = `Listed ${clean(baseName(path), 80)}`), (full = scrubText(path, secrets).slice(0, FULL_MAX));
      break;
    case "send_file":
      if (path) (detail = `Sent ${clean(baseName(path), 80)}`), (full = scrubText(path, secrets).slice(0, FULL_MAX));
      break;
    case "web_fetch":
    case "browser_navigate": {
      const host = hostOf(str(args["url"]));
      if (host) detail = `Opened ${clean(host, 80)}`;
      break;
    }
    case "computer_act": {
      const actions = Array.isArray(args["actions"]) ? (args["actions"] as unknown[]) : [];
      const first = actions[0];
      if (typeof first === "object" && first !== null) {
        const a = first as Record<string, unknown>;
        const kind = str(a["kind"]);
        const at = typeof a["x"] === "number" && typeof a["y"] === "number" ? ` at ${Math.round(a["x"] as number)}, ${Math.round(a["y"] as number)}` : "";
        const more = actions.length > 1 ? ` (+${actions.length - 1} more)` : "";
        if (kind === "click") detail = `${a["double"] === true ? "Double-clicked" : "Clicked"}${at}${more}`;
        else if (kind === "focus") detail = `Opened ${clean(str(a["application"]), 60)}${more}`;
        else if (kind === "type") detail = `Typed text${more}`;
        else if (kind === "key") detail = `Pressed ${clean(str(a["key"]), 30)}${more}`;
        else if (kind === "scroll") detail = `Scrolled${at}${more}`;
        else if (kind) detail = `${kind[0]?.toUpperCase() ?? ""}${kind.slice(1)}${at}${more}`;
      }
      break;
    }
    case "memory_search": {
      const query = str(args["query"]);
      if (query) detail = `Searched memory for "${clean(query, 70)}"`;
      break;
    }
    default:
      break;
  }
  // If the model gave a short reason for the call, it is shown. It is never a required argument.
  const why = clean(str(args["why"]) || str(args["reason"]), WHY_MAX);
  return { ...(detail ? { detail: detail.slice(0, DETAIL_MAX) } : {}), ...(full ? { full } : {}), ...(why ? { why } : {}) };
}

/** What a finished call left: the last lines of its output, its exit code, and for a failure the error line. */
export interface StepOutcome {
  tail?: string;
  exit?: number;
  error?: string;
}

/** Reads a run_terminal-like result (stdout, stderr, exitCode) or an error reply into a scrubbed outcome. */
export function outcomeOf(
  reply: { ok: true; result: Record<string, unknown> } | { ok: false; error: { code: string; message: string } },
  secrets: readonly string[] = [],
): StepOutcome {
  const out: StepOutcome = {};
  if (!reply.ok) {
    out.error = oneLine(scrubText(`${reply.error.message}`, secrets), ERROR_MAX);
    return out;
  }
  const result = reply.result;
  const stdout = str(result["stdout"]);
  const stderr = str(result["stderr"]);
  const code = result["exitCode"];
  if (typeof code === "number" && Number.isFinite(code)) out.exit = Math.trunc(code);
  const text = [stdout, stderr].filter((t) => t.trim() !== "").join("\n");
  if (text !== "") {
    const lines = scrubText(text, secrets)
      .split(/\r?\n/)
      .map((l) => l.replace(/\s+$/, ""))
      .filter((l, i, all) => l !== "" || i < all.length - 1);
    const tail = lines.slice(-TAIL_LINES).join("\n");
    out.tail = tail.length <= TAIL_MAX ? tail : tail.slice(tail.length - TAIL_MAX);
  }
  if ((out.exit !== undefined && out.exit !== 0) || result["ok"] === false) {
    const line = [...scrubText(stderr || stdout, secrets).split(/\r?\n/)].reverse().find((l) => l.trim() !== "");
    out.error = oneLine(line ?? str(result["error"]) ?? "Failed", ERROR_MAX) || "Failed";
  }
  return out;
}
