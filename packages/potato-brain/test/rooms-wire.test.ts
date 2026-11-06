// The wire between the app (Rust) and this brain, checked against one shared file:
// test/fixtures/room-wire.json. The Rust side (rooms/wire_tests.rs) builds the item and parses the events from the
// same file, so a change on either side that the other cannot read fails a test.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { PotatoRoomTransport } from "../src/rooms/transport.ts";
import { parseRoomItem } from "../src/rooms/parse.ts";

const fixture = JSON.parse(readFileSync(new URL("./fixtures/room-wire.json", import.meta.url), "utf8")) as {
  item: Record<string, unknown>;
  events: Record<string, unknown>[];
  personItem: Record<string, unknown>;
};

test("the brain reads the item the app writes", () => {
  const parsed = parseRoomItem(fixture.item);
  assert.ok(parsed);
  assert.equal(parsed.id, "l0000000000a1");
  assert.equal(parsed.conversationId, "app:room:r0123456789ab");
  assert.equal(parsed.facts.me.id, "nimbus");
  assert.equal(parsed.facts.roster.length, 3);
  assert.equal(parsed.facts.roster[0]?.kind, "person");
  assert.equal(parsed.facts.recent.length, 1);
  assert.equal(parsed.facts.recent[0]?.atMs, 1000);
  assert.equal(parsed.facts.line.atMs, 2000);
  assert.equal(parsed.facts.line.inReplyTo, null);
  assert.deepEqual(parsed.meta, { tools: "full", sandbox: true, sandboxOnly: false });
});

test("the events the brain writes are the events in the file the app reads", () => {
  const sent: Record<string, unknown>[] = [];
  const t = new PotatoRoomTransport((e) => sent.push({ v: 1, ...e }));
  const room = "r0123456789ab";
  t.typing(room, true);
  t.post({ roomId: room, inReplyTo: "l0000000000a1", segments: ["hello", "there"], kaomoji: "(^_^)", tone: "genki" });
  t.pass(room, "l0000000000a1", "low_score");
  t.fail(room, "l0000000000a1", "PROVIDER_LIMIT");
  assert.deepEqual(sent, fixture.events);
});

test("the brain reads the item for a bare hi from You (an item Rust generates for a send_room)", () => {
  const parsed = parseRoomItem(fixture.personItem);
  assert.ok(parsed, "a bare hi in a two-bot group must parse");
  assert.equal(parsed.facts.line.text, "hi");
  assert.equal(parsed.facts.line.from.name, "You");
  assert.equal(parsed.facts.line.from.id, "owner");
  assert.equal(parsed.facts.me.id, "blank-2");
  assert.equal(parsed.facts.roster.length, 3);
});
