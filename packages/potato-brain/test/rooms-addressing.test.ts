import { test } from "node:test";
import assert from "node:assert/strict";
import { addressedTo, aimedElsewhere } from "../src/rooms/addressing.ts";
import type { Member, RoomLine } from "../src/rooms/types.ts";

const ren: Member = { kind: "person", id: "owner", name: "Ren" };
const fish: Member = { kind: "bot", id: "rfish", name: "Fish" };
const potato: Member = { kind: "bot", id: "rpotato", name: "Potato" };
const bean: Member = { kind: "bot", id: "rbean", name: "Mr Bean" };
const roster = [ren, fish, potato, bean];

function line(id: string, text: string, from: Member = ren, inReplyTo: string | null = null): RoomLine {
  return { id, from, text, atMs: 1_000, hop: 0, inReplyTo };
}

test("at mention calls me", () => {
  assert.equal(addressedTo(line("l1", "@fish what time is it"), fish, roster, []), "me");
});

test("name at start calls me", () => {
  assert.equal(addressedTo(line("l1", "Fish, help"), fish, roster, []), "me");
});

test("name inside sentence does not call", () => {
  assert.equal(addressedTo(line("l1", "I like fish"), fish, roster, []), "none");
});

test("multi word name", () => {
  assert.equal(addressedTo(line("l1", "@mr bean hi"), bean, roster, []), "me");
});

test("everyone", () => {
  assert.equal(addressedTo(line("l1", "hi everyone"), potato, roster, []), "everyone");
});

test("everyone with punctuation", () => {
  assert.equal(addressedTo(line("l1", "Everyone!"), potato, roster, []), "everyone");
});

test("other bot", () => {
  assert.equal(addressedTo(line("l1", "@potato do it"), fish, roster, []), "other");
});

test("me and other", () => {
  assert.equal(addressedTo(line("l1", "@fish @potato both"), potato, roster, []), "me");
});

test("reply to my line", () => {
  const mine = line("l0", "here", fish);
  assert.equal(addressedTo(line("l1", "and then?", ren, "l0"), fish, roster, [mine]), "me");
});

test("plain question", () => {
  assert.equal(addressedTo(line("l1", "what should we eat?"), fish, roster, []), "none");
});

test("aimedElsewhere true", () => {
  assert.equal(aimedElsewhere(line("l1", "Potato: go"), fish, roster, []), true);
});

test("at mention needs a whole name: @fishing does not call Fish", () => {
  assert.equal(addressedTo(line("l1", "@fishing is fun"), fish, roster, []), "none");
  assert.equal(addressedTo(line("l1", "@Fish, hi"), fish, roster, []), "me");
  assert.equal(addressedTo(line("l1", "ask @fish."), fish, roster, []), "me");
  assert.equal(addressedTo(line("l1", "mail a@fish.com"), fish, roster, []), "none");
});

test("whole name works for names with spaces and for Chinese names", () => {
  assert.equal(addressedTo(line("l1", "@mr beanie hi"), bean, roster, []), "none");
  const xiaoyu: Member = { kind: "bot", id: "rxy", name: "小魚" };
  const xiaoyugan: Member = { kind: "bot", id: "rxyg", name: "小魚乾" };
  const zh = [ren, xiaoyu, xiaoyugan];
  assert.equal(addressedTo(line("l1", "@小魚你好"), xiaoyu, zh, []), "me");
  assert.equal(addressedTo(line("l1", "@小魚乾你好"), xiaoyu, zh, []), "other");
  assert.equal(addressedTo(line("l1", "@小魚乾你好"), xiaoyugan, zh, []), "me");
  assert.equal(addressedTo(line("l1", "請@小魚幫忙"), xiaoyu, zh, []), "me");
});
