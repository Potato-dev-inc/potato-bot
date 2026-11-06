import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Block } from "../types.ts";

const MAX_BULLETS = 12;
const MAX_CHARS = 700;
const MAX_NOTE_CHARS = 200;

const HEADER = [
  "# 她自己的筆記",
  "",
  "這是藍色大肥魚自己維護的部分：她學到關於自己、關於這個人的事。",
  "核心人格在 core.md，不給她改。",
  "",
].join("\n");

export function notesPath(personaDir: string): string {
  return join(personaDir, "notes.md");
}

function bullets(text: string): string[] {
  return text
    .split(/\r?\n/)
    .filter((line) => line.startsWith("- "))
    .slice(-MAX_BULLETS);
}

export function clipNotes(text: string): string {
  const body = [HEADER, ...bullets(text)].join("\n");
  return body.length > MAX_CHARS ? body.slice(body.length - MAX_CHARS) : body;
}

/**
 * Notes go into the volatile tip, not the cached prefix: writing one must
 * never change the prefix mid-session (that would bust the cache and trip
 * the prefix guard).
 */
export function loadNotes(personaDir: string): string {
  const path = notesPath(personaDir);
  if (!existsSync(path)) return "";
  try {
    const text = readFileSync(path, "utf8").trim();
    if (text === "") return "";
    return clipNotes(text);
  } catch {
    return "";
  }
}

export function notesBlock(personaDir: string, now = new Date()): Block | null {
  const notes = loadNotes(personaDir);
  if (notes === "") return null;

  // tail-token budget: keep the newest few bullets only
  const lines = notes
    .split(/\r?\n/)
    .filter((line) => line.startsWith("- "))
    .slice(-4)
    .map((line) => (line.length > 60 ? `${line.slice(0, 60)}…` : line));

  if (lines.length === 0) return null;

  return {
    id: "notes",
    volatility: 4,
    content: `<self>${lines.join("　")}</self>`,
  };
}

/** Append one line, keeping the newest and trimming the oldest. */
export function appendNote(
  personaDir: string,
  note: string,
  now = new Date(),
): string {
  const clean = note.replace(/\s+/g, " ").trim().slice(0, MAX_NOTE_CHARS);
  if (clean === "") return "";

  const path = notesPath(personaDir);
  const existing = existsSync(path) ? bullets(readFileSync(path, "utf8")) : [];
  const line = `- ${now.toISOString().slice(0, 10)} ${clean}`;
  const next = [HEADER, ...[...existing, line].slice(-MAX_BULLETS)].join("\n");

  mkdirSync(personaDir, { recursive: true });
  writeFileSync(path, `${next}\n`, "utf8");
  return line;
}

/** Where forgotten notes go: a plain text file, not `.md`, and nothing in the brain reads it back. */
export const NOTES_TRASH_FILE = "notes-trash.txt";

export function notesTrashPath(personaDir: string): string {
  return join(personaDir, NOTES_TRASH_FILE);
}

/** The longest text a forget call may give (the same as a note). */
export const FORGET_MATCH_MAX = MAX_NOTE_CHARS;

export type ForgetResult =
  | { status: "forgotten"; line: string }
  | { status: "missing" }
  | { status: "ambiguous"; count: number };

/**
 * Takes one note out of notes.md. `match` is a few words from it (case and spacing ignored). It must match exactly one
 * note: no match, or more than one, is refused and nothing is removed. The removed line is kept in notes-trash.txt
 * with the time it was forgotten, so it can be put back by hand. The notes file is where the bot's notes are kept,
 * so a forgotten note stops showing up for the bot from its next turn.
 */
export function forgetNote(personaDir: string, match: string, now = new Date()): ForgetResult {
  const needle = match.replace(/\s+/g, " ").trim().toLowerCase();
  const path = notesPath(personaDir);
  if (needle === "" || !existsSync(path)) return { status: "missing" };

  const lines = readFileSync(path, "utf8")
    .split(/\r?\n/)
    .filter((line) => line.startsWith("- "));
  const hits: number[] = [];
  lines.forEach((line, i) => {
    if (line.replace(/\s+/g, " ").toLowerCase().includes(needle)) hits.push(i);
  });
  if (hits.length === 0) return { status: "missing" };
  if (hits.length > 1) return { status: "ambiguous", count: hits.length };

  const at = hits[0] as number;
  const gone = lines[at] as string;
  const kept = lines.filter((_, i) => i !== at);
  mkdirSync(personaDir, { recursive: true });
  writeFileSync(path, `${[HEADER, ...kept].join("\n")}\n`, "utf8");
  appendFileSync(notesTrashPath(personaDir), `${now.toISOString()} ${gone}\n`, "utf8");
  return { status: "forgotten", line: gone };
}
