// The provider's second try: an empty or cut-off reply gets a bigger budget, a 5xx one more go; Stop and 4xx never wait.
import { test, afterEach } from "node:test";
import assert from "node:assert/strict";
import { OpenAICompatProvider } from "../src/core/openai.ts";
import { CodedError, isContextTooLong, reasonOf } from "../src/core/errors.ts";

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

type Step = Response | (() => Promise<Response>);
function fake(steps: Step[]): { bodies: Array<Record<string, unknown>>; calls: () => number } {
  const bodies: Array<Record<string, unknown>> = [];
  let i = 0;
  globalThis.fetch = (async (_url: unknown, init?: RequestInit) => {
    bodies.push(JSON.parse(String(init?.body)) as Record<string, unknown>);
    const step = steps[Math.min(i++, steps.length - 1)]!;
    return typeof step === "function" ? step() : step.clone();
  }) as typeof fetch;
  return { bodies, calls: () => i };
}
const json = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
const reply = (content: string, finish: string, completion = 5, extra: Record<string, unknown> = {}): Response =>
  json({
    choices: [{ message: { content, ...extra }, finish_reason: finish }],
    usage: { prompt_tokens: 10, completion_tokens: completion },
  });
const cutCall = (): Response =>
  reply("", "length", 7, { tool_calls: [{ id: "c1", function: { name: "run_terminal", arguments: '{"command":"ls' } }] });
const goodCall = (): Response =>
  reply("", "tool_calls", 3, { tool_calls: [{ id: "c1", function: { name: "run_terminal", arguments: '{"command":"ls"}' } }] });

function make(thinking: "auto" | "enabled" | "disabled" = "auto"): OpenAICompatProvider {
  const p = new OpenAICompatProvider({ apiKey: "k", baseUrl: "https://x.test/v1", model: "m", thinking });
  p.retryDelayMs = 1;
  return p;
}
const req = { messages: [{ role: "user" as const, content: "hi" }], maxTokens: 600 };

test("an empty reply is retried once with a larger budget and thinking off", async () => {
  const f = fake([reply("", "length", 600), reply("hello", "stop", 4)]);
  const res = await make("enabled").chat(req);
  assert.equal(res.text, "hello");
  assert.equal(f.calls(), 2);
  assert.equal(f.bodies[0]!["max_tokens"], 600);
  assert.equal(f.bodies[1]!["max_tokens"], 4000);
  assert.deepEqual(f.bodies[0]!["thinking"], { type: "enabled" });
  assert.deepEqual(f.bodies[1]!["thinking"], { type: "disabled" });
});

test("with thinking on auto the retry sends no thinking field", async () => {
  const f = fake([reply("", "length"), reply("ok", "stop")]);
  await make().chat(req);
  assert.equal("thinking" in f.bodies[1]!, false);
});

test("a cut-off tool call is never passed on; the retry's whole call is", async () => {
  const f = fake([cutCall(), goodCall()]);
  const res = await make().chat({ ...req, maxTokens: 4000 });
  assert.equal(f.calls(), 2);
  assert.equal(f.bodies[1]!["max_tokens"], 16000);
  assert.equal(res.toolCalls?.length, 1);
  assert.equal(res.toolCalls?.[0]?.arguments, '{"command":"ls"}');
});

test("a tool call cut off twice comes back flagged cutOff, so the loop can tell the model", async () => {
  const f = fake([cutCall(), cutCall()]);
  const res = await make().chat(req);
  assert.equal(f.calls(), 2);
  assert.equal(res.cutOff, true);
  assert.equal(res.toolCalls?.length, 1);
});

test("a 400 that says the context is too long fails with the fixed reason and none of the body", async () => {
  fake([json({ error: { message: "This model's maximum context length is 8192 tokens. secret-detail" } }, 400)]);
  await assert.rejects(make().chat(req), (e: unknown) => {
    assert.ok(e instanceof CodedError);
    assert.equal(isContextTooLong(e), true);
    assert.equal(reasonOf(e), "context too long");
    assert.ok(!String(e.message).includes("secret-detail"));
    return true;
  });
});

test("any other 400 is not a context error", async () => {
  fake([json({ error: { message: "bad parameter" } }, 400)]);
  await assert.rejects(make().chat(req), (e: unknown) => e instanceof CodedError && !isContextTooLong(e));
});

test("empty twice reports the finish reason", async () => {
  fake([reply("", "length"), reply("", "stop")]);
  await assert.rejects(make().chat(req), (e: unknown) => reasonOf(e) === "empty reply finish=stop");
});

test("a 503 is retried once and then works", async () => {
  const f = fake([new Response("secret body", { status: 503 }), reply("hi", "stop")]);
  const res = await make().chat(req);
  assert.equal(res.text, "hi");
  assert.equal(f.calls(), 2);
  assert.equal(f.bodies[1]!["max_tokens"], 600);
});

test("a 503 twice is PROVIDER_DOWN with reason http 503 and no body text", async () => {
  const f = fake([new Response("secret body sk-123", { status: 503 })]);
  await assert.rejects(make().chat(req), (e: unknown) => {
    assert.ok(e instanceof CodedError);
    assert.equal(e.code, "PROVIDER_DOWN");
    assert.equal(reasonOf(e), "http 503");
    assert.ok(!/secret|sk-123|x\.test/.test(e.message + (e.reason ?? "")));
    return true;
  });
  assert.equal(f.calls(), 2);
});

test("an unreadable reply is retried once", async () => {
  const f = fake([new Response("not json", { status: 200 }), reply("fine", "stop")]);
  assert.equal((await make().chat(req)).text, "fine");
  assert.equal(f.calls(), 2);
});

test("401, 402, 403 and 429 are not retried", async () => {
  for (const status of [401, 402, 403, 429]) {
    const f = fake([new Response("", { status })]);
    await assert.rejects(make().chat(req), (e: unknown) => e instanceof CodedError && e.status === status);
    assert.equal(f.calls(), 1, `status ${status}`);
  }
});

test("Stop during the retry pause ends it with no second call", async () => {
  const f = fake([new Response("", { status: 503 })]);
  const p = make();
  p.retryDelayMs = 60_000;
  const ctl = new AbortController();
  const pending = p.chat({ ...req, signal: ctl.signal });
  setTimeout(() => ctl.abort(), 20);
  await assert.rejects(pending, (e: unknown) => e instanceof CodedError);
  assert.equal(f.calls(), 1);
});

test("Stop already pressed before the retry also ends it", async () => {
  const ctl = new AbortController();
  const f = fake([() => {
    ctl.abort();
    return Promise.resolve(new Response("", { status: 500 }));
  }]);
  await assert.rejects(make().chat({ ...req, signal: ctl.signal }));
  assert.equal(f.calls(), 1);
});

test("usage counts both attempts of an empty-then-success retry", async () => {
  fake([reply("", "length", 600), reply("hello", "stop", 4)]);
  const res = await make().chat(req);
  assert.equal(res.usage.promptTokens, 20);
  assert.equal(res.usage.completionTokens, 604);
});
