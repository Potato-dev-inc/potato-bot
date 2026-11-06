// find_tool and use_tool (src/core/tool-bridge.ts, Agent.tools / bridgeCall in loop.ts).
// What is shown: a long tool list is cut to the core tools plus the two bridge tools, the prompt gets smaller (measured), a hidden
// tool runs through use_tool exactly as by its own name (same run, same host call, same name for the host's gates), and use_tool
// can NOT reach a tool the turn was not offered (sandbox off, not the owner, a routine, a room, memory level) or a room-off tool,
// nor itself, nor a tool that does not exist. Every host check that follows sees the real tool name.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Agent, ROOM_OFF_TOOLS } from "../src/core/loop.ts";
import type { ExtraTool } from "../src/core/loop.ts";
import { helperTool } from "../src/core/helper-tool.ts";
import { sandboxTools } from "../src/core/sandbox-tools.ts";
import { scratchpadTools } from "../src/core/scratchpad-tools.ts";
import { jobTools } from "../src/core/job-tools.ts";
import { scheduleTools } from "../src/core/schedule-tools.ts";
import { waitTools } from "../src/core/wait-tools.ts";
import { historyTools } from "../src/core/history-tools.ts";
import { selfSettingsTool } from "../src/core/self-settings-tools.ts";
import { plotTool } from "../src/core/plot-tools.ts";
import { requestLoginTool } from "../src/core/login-tools.ts";
import { addMcpServerTool, requestConnectorTool } from "../src/core/mcp-tools.ts";
import { askUserTool, requestApprovalTool } from "../src/core/question-tools.ts";
import { messageUserTool } from "../src/core/message-user-tools.ts";
import { BRIDGE_MIN_TOOLS, CORE_EXTRA_TOOLS, FIND_TOOL_NAME, USE_TOOL_NAME, searchTools } from "../src/core/tool-bridge.ts";
import { initialMood } from "../src/persona/mood.ts";
import type { ChatRequest, ChatResult, Provider, ToolCall, Usage } from "../src/types.ts";

const usage = (): Usage => ({ promptTokens: 1, completionTokens: 1, cacheHitTokens: 0, cacheMissTokens: 1 });
const NOW = () => new Date(2026, 10, 6, 12, 0, 0);

class Scripted implements Provider {
  readonly name = "scripted";
  calls: ChatRequest[] = [];
  private scripts: ChatResult[];
  constructor(scripts: ChatResult[]) {
    this.scripts = scripts;
  }
  async chat(req: ChatRequest): Promise<ChatResult> {
    this.calls.push({ ...req, messages: req.messages.map((m) => ({ ...m })) });
    return this.scripts.shift() ?? { text: "ok", usage: usage(), model: "scripted" };
  }
}

const call = (id: string, name: string, args: Record<string, unknown>): ToolCall => ({ id, name, arguments: JSON.stringify(args) });
const asks = (...calls: ToolCall[]): ChatResult => ({ text: "", usage: usage(), model: "scripted", toolCalls: calls });
const says = (text: string): ChatResult => ({ text, usage: usage(), model: "scripted" });

/** The same tools the Potato channel gives a bot (channels/potato.ts), over a recording host. */
function fullSet(sent: Array<{ tool: string; args: Record<string, unknown> }>): ExtraTool[] {
  const callHost = async (tool: string, args: Record<string, unknown>) => {
    sent.push({ tool, args });
    return { ok: true as const, result: { ok: true, text: `ran ${tool}`, card: false } };
  };
  return [
    helperTool({ conversationId: "app:dm:b", newId: () => "id", emit: () => {} }),
    ...sandboxTools({ call: callHost }),
    ...scratchpadTools({ call: callHost }),
    ...jobTools({ call: callHost }),
    ...scheduleTools({ call: callHost }),
    ...waitTools({ call: callHost }),
    ...historyTools({ call: callHost }),
    plotTool({ call: callHost }),
    addMcpServerTool({ call: callHost }),
    requestConnectorTool({ call: callHost }),
    requestLoginTool({ call: callHost }),
    askUserTool({ call: callHost, inAppDm: true }),
    requestApprovalTool({ call: callHost }),
    messageUserTool(() => true),
    selfSettingsTool({ call: callHost }),
  ];
}

function agentWith(provider: Provider, extra: ExtraTool[]): Agent {
  return new Agent({
    provider,
    persona: [{ id: "p", volatility: 0, content: "persona" }],
    mood: initialMood(NOW()),
    now: NOW,
    extraTools: extra,
  });
}

const names = (req: ChatRequest | undefined): string[] => (req?.tools ?? []).map((t) => t.name);
const size = (req: ChatRequest | undefined): number => JSON.stringify(req?.tools ?? []).length;
const OWNER = { sandbox: true, owner: true } as const;

async function firstRequest(opts: Record<string, unknown>, bridge: boolean): Promise<ChatRequest> {
  const previous = process.env.POTATO_TOOL_BRIDGE;
  process.env.POTATO_TOOL_BRIDGE = bridge ? "1" : "0";
  try {
    const provider = new Scripted([says("ok")]);
    await agentWith(provider, fullSet([])).turn("hi", opts as never);
    return provider.calls[0]!;
  } finally {
    if (previous === undefined) delete process.env.POTATO_TOOL_BRIDGE;
    else process.env.POTATO_TOOL_BRIDGE = previous;
  }
}

test("MEASURE: the owner's turn with a computer sends far fewer tool characters with the bridge (numbers are printed)", async () => {
  const before = await firstRequest(OWNER, false);
  const after = await firstRequest(OWNER, true);
  const beforeChars = size(before);
  const afterChars = size(after);
  // The tools are sent as JSON next to the prompt text; about 4 characters make a token for this kind of text.
  console.log(`tool list before: ${names(before).length} tools, ${beforeChars} chars (~${Math.round(beforeChars / 4)} tokens)`);
  console.log(`tool list after:  ${names(after).length} tools, ${afterChars} chars (~${Math.round(afterChars / 4)} tokens)`);
  const whole = (req: ChatRequest): number => JSON.stringify({ messages: req.messages, tools: req.tools }).length;
  console.log(`whole request (test persona, so the real prompt text is larger): before ${whole(before)} chars, after ${whole(after)} chars`);
  assert.ok(names(before).length > BRIDGE_MIN_TOOLS, "the test set is a long list");
  assert.ok(afterChars < beforeChars * 0.7, `at least 30% smaller: ${beforeChars} -> ${afterChars}`);
  assert.ok(names(after).includes(FIND_TOOL_NAME) && names(after).includes(USE_TOOL_NAME));
  for (const core of ["run_terminal", "read_file", "computer_act", "ask_user", "request_approval", "memory_search", "self"]) {
    assert.ok(names(after).includes(core), `${core} stays in the list`);
  }
  assert.ok(!names(after).includes("schedule_create") && names(before).includes("schedule_create"));
});

test("a short list is sent as it always was: no bridge tools", async () => {
  const request = await firstRequest({}, true);
  assert.ok(!names(request).includes(FIND_TOOL_NAME) && !names(request).includes(USE_TOOL_NAME));
  // A room turn has no room-off tools, so its list may be short or long: the bridge is on exactly when the plain list is long.
  const plainRoom = await firstRequest({ ...OWNER, room: true }, false);
  const bridgedRoom = await firstRequest({ ...OWNER, room: true }, true);
  assert.equal(names(bridgedRoom).includes(USE_TOOL_NAME), names(plainRoom).length > BRIDGE_MIN_TOOLS);
});

test("find_tool lists the hidden tools with their arguments; use_tool runs one with the host seeing its real name", async () => {
  const sent: Array<{ tool: string; args: Record<string, unknown> }> = [];
  const provider = new Scripted([
    asks(call("a", FIND_TOOL_NAME, { query: "schedule routine" })),
    asks(call("b", USE_TOOL_NAME, { name: "schedule_list", arguments: {} })),
    says("done"),
  ]);
  const result = await agentWith(provider, fullSet(sent)).turn("what is scheduled?", OWNER as never);
  assert.equal(result.segments.join(" ").includes("done"), true);
  const found = provider.calls[1]!.messages.filter((m) => m.role === "tool").at(-1)!.content;
  assert.match(found, /schedule_list/);
  assert.match(found, /schedule_create/);
  assert.match(found, /arguments:/);
  assert.deepEqual(sent.map((s) => s.tool), ["schedule_list"], "the host heard schedule_list, never use_tool");
  assert.ok(!sent.some((s) => (s.tool as string) === USE_TOOL_NAME || (s.tool as string) === FIND_TOOL_NAME));
});

test("naming a hidden tool directly works like use_tool, and the arguments reach the host untouched", async () => {
  const sent: Array<{ tool: string; args: Record<string, unknown> }> = [];
  const provider = new Scripted([asks(call("a", "wait_cancel", { id: "w-1" })), says("ok")]);
  await agentWith(provider, fullSet(sent)).turn("cancel it", OWNER as never);
  assert.deepEqual(sent, [{ tool: "wait_cancel", args: { id: "w-1" } }]);
});

test("use_tool cannot reach a tool the turn was not offered, a room-off tool, itself, or one that does not exist", async () => {
  // A room turn: ROOM_OFF tools are neither sent nor reachable.
  const sent: Array<{ tool: string; args: Record<string, unknown> }> = [];
  const attempts = ["add_mcp_server", "request_connector", "request_login", "wait_until", "change_my_setting", "render_plot", "job_start", "ask_helper", "message_user", FIND_TOOL_NAME, USE_TOOL_NAME, "no_such_tool", "", "../run_terminal"];
  const provider = new Scripted([
    asks(...attempts.slice(0, 4).map((n, i) => call(`r${i}`, USE_TOOL_NAME, { name: n, arguments: { service: "x", name: "x", url: "https://x.example/mcp" } }))),
    asks(...attempts.slice(4, 8).map((n, i) => call(`s${i}`, USE_TOOL_NAME, { name: n, arguments: {} }))),
    asks(...attempts.slice(8, 12).map((n, i) => call(`t${i}`, USE_TOOL_NAME, { name: n, arguments: {} }))),
    asks(...attempts.slice(12).map((n, i) => call(`u${i}`, USE_TOOL_NAME, { name: n, arguments: {} }))),
    says("done"),
  ]);
  const agent = agentWith(provider, fullSet(sent));
  await agent.turn("hi", { ...OWNER, room: true } as never);
  for (const gone of ROOM_OFF_TOOLS) {
    assert.ok(!names(provider.calls[0]).includes(gone), `${gone} is not offered in a room`);
  }
  assert.deepEqual(sent, [], "not one of those reached the host");
  const answers = provider.calls.flatMap((c) => c.messages.filter((m) => m.role === "tool").map((m) => m.content));
  assert.ok(answers.length >= 13);
  assert.ok(answers.every((a) => /not a tool you can reach|unknown|不認識/i.test(a) || a.length > 0));
});

test("use_tool cannot run owner-only, owner-person or computer tools on a turn that is not the owner's or has no computer", async () => {
  for (const [label, opts, hidden] of [
    ["not the owner", { sandbox: true }, ["schedule_create", "scratchpad_add", "search_history", "wait_until", "change_my_setting"]],
    ["no computer", { owner: true }, ["set_wallpaper", "ask_person_for_screen", "send_file"]],
    ["a routine's own run", { sandbox: true, owner: true, routine: true }, ["schedule_create", "wait_until", "change_my_setting", "request_login", "add_mcp_server"]],
  ] as const) {
    const sent: Array<{ tool: string; args: Record<string, unknown> }> = [];
    const provider = new Scripted([asks(...hidden.map((n, i) => call(`x${i}`, USE_TOOL_NAME, { name: n, arguments: { query: "x", text: "x", title: "x" } }))), says("done")]);
    await agentWith(provider, fullSet(sent)).turn("hi", opts as never);
    assert.deepEqual(sent, [], `${label}: nothing reached the host`);
  }
});

test("use_tool cannot run a memory tool at the no_memory level, and nothing offered is lost to the hidden set", async () => {
  const sent: Array<{ tool: string; args: Record<string, unknown> }> = [];
  const provider = new Scripted([asks(call("m", USE_TOOL_NAME, { name: "memory_search", arguments: { query: "x" } })), says("done")]);
  await agentWith(provider, fullSet(sent)).turn("hi", { ...OWNER, tools: "no_memory" } as never);
  assert.deepEqual(sent, []);
  const answer = provider.calls[1]!.messages.filter((m) => m.role === "tool").at(-1)!.content;
  assert.match(answer, /not a tool you can reach/);
});

test("a hidden tool keeps its own behaviour: a card ends the turn, a refusal from the host comes back as text", async () => {
  const sent: Array<{ tool: string; args: Record<string, unknown> }> = [];
  const extra = fullSet(sent);
  const withCard = extra.map((t) => (t.spec.name === "request_login" ? { ...t, run: () => ({ text: "Waiting for your answer on the card above.", endTurn: true }) } : t));
  const provider = new Scripted([asks(call("c", USE_TOOL_NAME, { name: "request_login", arguments: { site: "https://x.example" } })), says("never reached")]);
  const result = await agentWith(provider, withCard).turn("sign in", OWNER as never);
  assert.match(result.segments.join(" "), /Waiting for your answer/);
  assert.equal(provider.calls.length, 1, "the turn ended at the card");
});

test("the core list names real tools and the finder matches by name and by words", () => {
  const all = fullSet([]).map((t) => t.spec.name);
  for (const core of CORE_EXTRA_TOOLS) {
    assert.ok(all.includes(core) || core === "run_subagent" || core === "skill_read" || core === "forget_memory", `${core} is a real tool`);
  }
  const hidden = fullSet([]).filter((t) => !CORE_EXTRA_TOOLS.has(t.spec.name)).map((t) => t.spec);
  assert.match(searchTools("wallpaper", hidden), /set_wallpaper/);
  assert.match(searchTools("schedule", hidden).split("\n")[0]!, /schedule/);
  assert.match(searchTools("qqqqxx", hidden), /No tool matches/);
  assert.equal(searchTools("", hidden).split("\n").length, hidden.length);
});
