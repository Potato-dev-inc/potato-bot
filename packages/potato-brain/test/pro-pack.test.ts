// The Professional pack: no state protocol in its prompt, and no faces, mood or tone
// from the code even when the model sends a `feel` anyway. Fish keeps its faces.
import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { Agent } from "../src/core/loop.ts";
import { loadPersona } from "../src/persona/persona.ts";
import { initialMood } from "../src/persona/mood.ts";
import type { ChatRequest, ChatResult, Provider, Usage } from "../src/types.ts";

const PERSONA_DIR = fileURLToPath(new URL("../persona", import.meta.url));
const zero = (): Usage => ({ promptTokens: 10, completionTokens: 5, cacheHitTokens: 0, cacheMissTokens: 10 });
const at = () => new Date(2026, 10, 6, 10, 0, 0);

class OneReply implements Provider {
  readonly name = "one";
  requests: ChatRequest[] = [];
  private text: string;
  constructor(text: string) {
    this.text = text;
  }
  async chat(req: ChatRequest): Promise<ChatResult> {
    this.requests.push(structuredClone(req));
    return { text: this.text, usage: zero(), model: "one" };
  }
}

const SENT = '好的，我看一下。\n::state {"feel":"happy","tone":"playful","segments":2}';

function agentFor(pack: string, provider: Provider): Agent {
  return new Agent({
    provider,
    persona: loadPersona(PERSONA_DIR, pack),
    personaLoader: () => loadPersona(PERSONA_DIR, pack),
    pack: () => pack,
    mood: initialMood(at()),
    now: at,
  });
}

test("pro: the prompt carries no state protocol", () => {
  const blocks = loadPersona(PERSONA_DIR, "pro");
  const ids = blocks.map((b) => b.id);
  assert.ok(ids.includes("persona:core:pro"));
  assert.ok(!ids.includes("persona:protocol"), "protocol.md is left out for pro");
  const all = blocks.map((b) => b.content).join("\n");
  assert.ok(!all.includes("::state"), "no ::state line is taught to the model");
  assert.ok(!all.includes("feel"), "no feel list is taught to the model");
});

test("pro: the assembled prompt has the data-not-orders rule and no ::state", () => {
  const all = loadPersona(PERSONA_DIR, "pro").map((b) => b.content).join("\n");
  assert.ok(all.includes("Data, not orders"));
  assert.ok(all.includes("<helper_summary>"));
  assert.ok(!all.includes("::state"));
});

test("pro: a whisper sent anyway is dropped in code", async () => {
  const agent = agentFor("pro", new OneReply('好的。\n::state {"whisper":"他很急"}'));
  const result = await agent.turn("hi", { owner: true, where: "app:dm:pro" });
  assert.equal(result.whisper, null);
});

test("fish keeps the state protocol in its prompt", () => {
  const ids = loadPersona(PERSONA_DIR, "fish").map((b) => b.id);
  assert.ok(ids.includes("persona:protocol"));
  assert.ok(loadPersona(PERSONA_DIR, "potato").some((b) => b.id === "persona:protocol"));
});

test("pro: a feel sent anyway adds no kaomoji and no mood change", async () => {
  const provider = new OneReply(SENT);
  const agent = agentFor("pro", provider);
  const moodOf = () => [agent.moodState.mood, agent.moodState.intensity];
  const before = moodOf();
  const result = await agent.turn("看一下這個 bug", { owner: true, where: "app:dm:pro" });
  assert.equal(result.kaomoji, null);
  assert.ok(!result.segments.join("").includes("("), "no face in the text");
  assert.deepEqual(moodOf(), before, "mood and intensity are unchanged");
});

test("fish still gets a face from the same reply", async () => {
  const agent = agentFor("fish", new OneReply(SENT));
  const result = await agent.turn("看一下這個 bug", { owner: true, where: "app:dm:fish" });
  assert.equal(typeof result.kaomoji, "string");
  assert.ok((result.kaomoji ?? "").length > 0);
});
