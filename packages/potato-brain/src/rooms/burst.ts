import type { RoomLine } from "./types.ts";

export const BURST_WINDOW_MS = 10 * 60_000;
export const BURST_MAX_LINES = 12;

/** The lines a bot looks at when deciding. Pure: callers pass `now`. */
export class Burst {
  private lines: RoomLine[] = [];
  add(line: RoomLine, now: number): void {
    this.lines.push(line);
    this.prune(now);
  }
  /** Oldest first. */
  view(now: number): RoomLine[] { this.prune(now); return [...this.lines]; }
  private prune(now: number): void {
    this.lines = this.lines.filter((l) => now - l.atMs < BURST_WINDOW_MS);
    if (this.lines.length > BURST_MAX_LINES) this.lines = this.lines.slice(-BURST_MAX_LINES);
  }
}
