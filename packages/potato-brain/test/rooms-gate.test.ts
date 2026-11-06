import { test } from "node:test";
import assert from "node:assert/strict";
import { firstResponder, makeGate } from "../src/rooms/gate.ts";
import { SilenceBudget } from "../src/rooms/silence.ts";
import type { Member, RoomFacts, RoomLine, Verdict } from "../src/rooms/types.ts";

const ren: Member = { kind: "person", id: "owner", name: "Ren" };
const fish: Member = { kind: "bot", id: "rfish", name: "Fish" };
const potato: Member = { kind: "bot", id: "rpotato", name: "Potato" };
const bean: Member = { kind: "bot", id: "rbean", name: "Bean" };
const roster = [ren, fish, potato, bean];
const NOW = 1_000_000;
const gate = makeGate("heuristic");

function line(id: string, text: string, from: Member = ren, hop = 0, atMs = NOW): RoomLine {
  return { id, from, text, atMs, hop, inReplyTo: null };
}
function facts(l: RoomLine, me: Member, recent: RoomLine[] = []): RoomFacts {
  return { roomId: "r0123456789ab", roomName: "Weekend", me, roster, recent, line: l };
}
const decide = (f: RoomFacts, silence = new SilenceBudget()): Verdict => gate(f, { silence, now: NOW });

test("own line passes", () => {
  assert.deepEqual(decide(facts(line("l1", "hello there", fish, 1), fish)), { kind: "pass", why: "own_line" });
});

test("hop limit", () => {
  assert.deepEqual(decide(facts(line("l1", "@fish go", potato, 3), fish)), { kind: "pass", why: "hop_limit" });
  assert.equal(decide(facts(line("l1", "@fish go", potato, 2), fish)).kind, "speak");
});

test("called speaks even when budget spent", () => {
  const s = new SilenceBudget();
  for (let i = 0; i < 12; i++) s.spend(NOW - 100_000 - i * 1000);
  assert.deepEqual(decide(facts(line("l1", "@fish hi"), fish), s), { kind: "speak", why: "addressed" });
});

test("bot line not addressed passes", () => {
  assert.deepEqual(decide(facts(line("l1", "nice idea", potato, 1), fish)), { kind: "pass", why: "bot_not_addressed" });
});

test("bot handoff speaks", () => {
  assert.deepEqual(decide(facts(line("l1", "@fish can you take this?", potato, 1), fish)), { kind: "speak", why: "addressed" });
});

test("bot saying everyone does not wake the others", () => {
  assert.deepEqual(decide(facts(line("l1", "hi everyone", potato, 1), fish)), { kind: "pass", why: "bot_not_addressed" });
});

test("everyone speaks", () => {
  assert.deepEqual(decide(facts(line("l1", "hi everyone"), fish)), { kind: "speak", why: "everyone" });
});

test("aimed elsewhere passes", () => {
  assert.deepEqual(decide(facts(line("l1", "@potato go"), fish)), { kind: "pass", why: "aimed_elsewhere" });
});

test("only first responder answers open question", () => {
  const l = line("l-question", "what should we eat?");
  const out = [fish, potato, bean].map((b) => decide(facts(l, b)));
  assert.equal(out.filter((v) => v.kind === "speak" && v.why === "question").length, 1);
  assert.equal(out.filter((v) => v.kind === "pass" && v.why === "not_first_responder").length, 2);
});

test("first responder is stable", () => {
  assert.equal(firstResponder("l-x", roster), firstResponder("l-x", roster));
  assert.equal(firstResponder("l-x", [ren]), null);
});

test("left alone raises the bar", () => {
  const id = "l-q2";
  const me = roster.find((m) => m.id === firstResponder(id, roster))!;
  const shh = line("l0", "shh please", ren, 0, NOW - 60_000);
  assert.deepEqual(decide(facts(line(id, "what now?"), me, [shh])), { kind: "pass", why: "left_alone" });
});

test("an unaddressed line from the person, even a bare hi, is answered by the first responder only", () => {
  const id = "l-ok";
  const me = roster.find((m) => m.id === firstResponder(id, roster))!;
  assert.deepEqual(decide(facts(line(id, "hi"), me)), { kind: "speak", why: "chatty" });
  assert.deepEqual(decide(facts(line(id, "that is fine"), me)), { kind: "speak", why: "chatty" });
  const other = roster.find((m) => m.kind === "bot" && m.id !== me.id)!;
  assert.deepEqual(decide(facts(line(id, "hi"), other)), { kind: "pass", why: "not_first_responder" });
});

test("a plain statement from a bot still passes", () => {
  const id = "l-ok2";
  const me = roster.find((m) => m.id === firstResponder(id, roster))!;
  const bot = roster.find((m) => m.kind === "bot" && m.id !== me.id)!;
  assert.deepEqual(decide(facts({ ...line(id, "that is fine", bot, 1), inReplyTo: null }, me)), { kind: "pass", why: "bot_not_addressed" });
});

test("cooldown never blocks the person's line", () => {
  const id = "l-cd";
  const me = roster.find((m) => m.id === firstResponder(id, roster))!;
  const s = new SilenceBudget();
  s.spend(NOW - 5000);
  assert.deepEqual(decide(facts(line(id, "what now?"), me), s), { kind: "speak", why: "question" });
});
