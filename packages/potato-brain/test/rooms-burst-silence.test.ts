import { test } from "node:test";
import assert from "node:assert/strict";
import { Burst, BURST_MAX_LINES } from "../src/rooms/burst.ts";
import { SilenceBudget, SILENCE_CAP, SILENCE_WINDOW_MS } from "../src/rooms/silence.ts";
import type { RoomLine } from "../src/rooms/types.ts";

const ren = { kind: "person" as const, id: "owner", name: "Ren" };

function line(n: number, atMs: number): RoomLine {
  return { id: "l" + n, from: ren, text: "msg " + n, atMs, hop: 0, inReplyTo: null };
}

test("burst keeps 12", () => {
  const b = new Burst();
  const now = 1_000;
  for (let i = 1; i <= 15; i++) b.add(line(i, now), now);
  const v = b.view(now);
  assert.equal(v.length, BURST_MAX_LINES);
  assert.equal(v[0]?.id, "l4");
});

test("burst drops old", () => {
  const b = new Burst();
  b.add(line(1, 0), 0);
  const t11 = 11 * 60_000;
  b.add(line(2, t11), t11);
  assert.deepEqual(b.view(t11).map((l) => l.id), ["l2"]);
});

test("silence allows first", () => {
  assert.equal(new SilenceBudget().blocked(0), null);
});

test("cooldown after spend", () => {
  const s = new SilenceBudget();
  s.spend(0);
  assert.equal(s.blocked(10_000), "cooldown");
});

test("cooldown ends", () => {
  const s = new SilenceBudget();
  s.spend(0);
  assert.equal(s.blocked(20_000), null);
});

test("budget cap", () => {
  const s = new SilenceBudget();
  for (let i = 0; i < SILENCE_CAP; i++) s.spend(i * 30_000);
  assert.equal(s.blocked(SILENCE_CAP * 30_000), "budget");
});

test("budget window slides", () => {
  const s = new SilenceBudget();
  for (let i = 0; i < SILENCE_CAP; i++) s.spend(i * 30_000);
  assert.equal(s.blocked(SILENCE_WINDOW_MS + 1), null);
});
