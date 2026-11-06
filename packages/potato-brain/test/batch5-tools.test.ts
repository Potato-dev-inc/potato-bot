import { test } from "node:test";
import assert from "node:assert/strict";
import { Agent } from "../src/core/loop.ts";
import type { ExtraTool } from "../src/core/loop.ts";
import { askUserTool, ASK_USER_NAME } from "../src/core/question-tools.ts";
import { messageUserTool, MESSAGE_USER_NAME, MESSAGE_USER_TURN_CAP } from "../src/core/message-user-tools.ts";
import { initialMood } from "../src/persona/mood.ts";
import type { ChatRequest, ChatResult, Provider, Usage } from "../src/types.ts";

const zero = (): Usage => ({ promptTokens: 10, completionTokens: 5, cacheHitTokens: 0, cacheMissTokens: 10 });
const at = () => new Date(2026, 10, 5, 10, 0, 0);

class ScriptedProvider implements Provider {
  readonly name = "scripted";
  calls: ChatRequest[] = [];
  private scripts: ChatResult[];
  constructor(scripts: ChatResult[]) {
    this.scripts = scripts;
  }
  async chat(req: ChatRequest): Promise<ChatResult> {
    this.calls.push(structuredClone({ ...req, tools: req.tools }));
    return this.scripts.shift() ?? { text: "……", usage: zero(), model: "scripted" };
  }
}

function agentWith(provider: Provider, extra: ExtraTool[] = []): Agent {
  return new Agent({
    provider,
    persona: [{ id: "p", volatility: 0, content: "你是魚。" }],
    mood: initialMood(at()),
    now: at,
    extraTools: extra,
  });
}

const toolNames = (req: ChatRequest | undefined): string[] => (req?.tools ?? []).map((t) => t.name);

test("ask_user: the turn ends with the question, no second model round", async () => {
  const provider = new ScriptedProvider([
    {
      text: "",
      usage: zero(),
      model: "scripted",
      toolCalls: [{ id: "q1", name: ASK_USER_NAME, arguments: JSON.stringify({ question: "哪一天比較好？", choices: ["週五", "週六"] }) }],
    },
  ]);
  const agent = agentWith(provider, [askUserTool()]);
  const result = await agent.turn("幫我訂位", { owner: true, where: "app:dm:fish" });
  assert.equal(provider.calls.length, 1, "the question ends the turn");
  assert.ok(result.segments.join("").includes("哪一天比較好？"));
  assert.ok(result.segments.join("").includes("週五 / 週六"));
});

test("ask_user: offered to the owner, never on a routine's run or a stranger's turn", async () => {
  const provider = new ScriptedProvider([]);
  const agent = agentWith(provider, [askUserTool()]);
  await agent.turn("hi", { owner: true, where: "app:dm:fish" });
  await agent.turn("hi", { owner: true, routine: true, where: "app:dm:fish" });
  await agent.turn("hi", { where: "app:dm:fish" });
  assert.ok(toolNames(provider.calls[0]).includes(ASK_USER_NAME));
  assert.ok(!toolNames(provider.calls[1]).includes(ASK_USER_NAME));
  assert.ok(!toolNames(provider.calls[2]).includes(ASK_USER_NAME));
});

test("ask_user: an empty question is refused", () => {
  const out = askUserTool().run({ question: "  " }, { turn: 1, where: undefined, model: "m" });
  assert.equal(out, "Write the question first.");
});

test("message_user: at most the per-reply cap, and the count resets on the next reply", () => {
  const sent: string[] = [];
  const tool = messageUserTool((text) => {
    sent.push(text);
    return true;
  });
  const ctx = (turn: number) => ({ turn, where: undefined, model: "m" });
  for (let i = 0; i < MESSAGE_USER_TURN_CAP + 2; i++) tool.run({ text: `step ${i}` }, ctx(1));
  assert.equal(sent.length, MESSAGE_USER_TURN_CAP);
  assert.equal(tool.spec.name, MESSAGE_USER_NAME);
  tool.run({ text: "next reply" }, ctx(2));
  assert.equal(sent.length, MESSAGE_USER_TURN_CAP + 1);
});

test("message_user: empty text and a failed send send nothing and count nothing", () => {
  let fail = true;
  const sent: string[] = [];
  const tool = messageUserTool((text) => {
    if (fail) return false;
    sent.push(text);
    return true;
  });
  const ctx = { turn: 1, where: undefined, model: "m" };
  assert.equal(tool.run({ text: "" }, ctx), "Nothing to send.");
  assert.equal(tool.run({ text: "hi" }, ctx), "The message could not be sent right now.");
  fail = false;
  for (let i = 0; i < MESSAGE_USER_TURN_CAP; i++) tool.run({ text: "hi" }, ctx);
  assert.equal(sent.length, MESSAGE_USER_TURN_CAP, "the failed send did not use up the cap");
});

test("run_subagent: a helper turn gets only the read-only lookups, and its answer comes back fenced", async () => {
  const provider = new ScriptedProvider([
    {
      text: "",
      usage: zero(),
      model: "scripted",
      toolCalls: [{ id: "s1", name: "run_subagent", arguments: JSON.stringify({ task: "find the cafe name" }) }],
    },
    { text: "", usage: zero(), model: "scripted", toolCalls: [{ id: "m1", name: "memory_search", arguments: JSON.stringify({ query: "cafe" }) }] },
    { text: "The cafe is Blue Door.", usage: zero(), model: "scripted" },
    { text: "It is Blue Door.", usage: zero(), model: "scripted" },
  ]);
  const agent = agentWith(provider);
  const result = await agent.turn("where did we eat", { owner: true, where: "app:dm:fish" });
  assert.equal(result.segments.join(""), "It is Blue Door.");
  const parent = toolNames(provider.calls[0]);
  assert.ok(parent.includes("run_subagent"));
  const helper = toolNames(provider.calls[1]);
  assert.deepEqual(helper.sort(), ["memory_open", "memory_search"]);
  assert.ok(!helper.includes("run_subagent"), "no nesting");
  const last = provider.calls[3]?.messages.find((m) => m.role === "tool");
  assert.ok(last, "the parent sees the helper result as a tool message");
  assert.ok(last.content.includes("<subagent_result>") && last.content.includes("Blue Door"));
});

test("run_subagent: at most 3 helper turns per reply", async () => {
  const call = (id: string) => ({ id, name: "run_subagent", arguments: JSON.stringify({ task: `task ${id}` }) });
  const provider = new ScriptedProvider([
    { text: "", usage: zero(), model: "scripted", toolCalls: [call("a"), call("b"), call("c"), call("d")] },
    { text: "one", usage: zero(), model: "scripted" },
    { text: "two", usage: zero(), model: "scripted" },
    { text: "three", usage: zero(), model: "scripted" },
    { text: "done", usage: zero(), model: "scripted" },
  ]);
  const agent = agentWith(provider);
  await agent.turn("go", { owner: true, where: "app:dm:fish" });
  assert.equal(provider.calls.length, 5, "the fourth helper never starts a model call");
  const fourth = provider.calls[4]?.messages.filter((m) => m.role === "tool").at(-1);
  assert.ok(fourth?.content.includes("already used"), fourth?.content);
});

test("run_subagent: not offered to a stranger", async () => {
  const provider = new ScriptedProvider([]);
  const agent = agentWith(provider);
  await agent.turn("hi", { where: "app:dm:fish" });
  assert.ok(!toolNames(provider.calls[0]).includes("run_subagent"));
});
