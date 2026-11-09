import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Agent } from "../src/core/loop.ts";
import type { ChatRequest, ChatResult, Provider, Usage } from "../src/types.ts";
import { readablePage, sandboxTools, sandboxToolSpecs, SANDBOX_TOOL_NAMES } from "../src/core/sandbox-tools.ts";
import { pageText } from "../src/core/web-text.ts";
import { initialMood } from "../src/persona/mood.ts";
import { VaultMemory } from "../src/memory/vaultMemory.ts";

const CTX = { turn: 1, where: undefined, model: "m" };
const usage = (): Usage => ({ promptTokens: 1, completionTokens: 1, cacheHitTokens: 0, cacheMissTokens: 1 });
const NOW = () => new Date(2026, 10, 5, 9, 0, 0);
const EM = "—";

test("the page text keeps the title and the words, drops scripts and the head, and decodes entities", () => {
  const html =
    '<!doctype html><html><head><title>Ex &amp; Co</title><style>p{color:red}</style></head>' +
    '<body><script>alert("no")</script><h1>Hello</h1><p>Tom &amp; Jerry ' + EM + ' &lt;b&gt;</p><!-- hidden --><div>Bye</div></body></html>';
  const page = pageText(html, "text/html; charset=utf-8");
  assert.equal(page.title, "Ex & Co");
  assert.equal(page.text, "Hello\n\nTom & Jerry " + EM + " <b>\n\nBye", "a blank line between blocks");
  assert.equal(page.note, undefined);
});

test("a plain text page is passed through as text, and a picture or a file is a note, not bytes", () => {
  assert.deepEqual(pageText("line one\n\n\n\nline two", "text/plain"), { title: "", text: "line one\n\nline two" });
  const pic = pageText("\u0089PNG", "image/png");
  assert.equal(pic.text, "");
  assert.match(pic.note ?? "", /not a readable text page \(image\/png\)/);
});

test("readablePage turns the app's answer into the title and the words, and drops the raw body", () => {
  const out = readablePage({ ok: true, url: "https://example.com/", contentType: "text/html", body: "<title>Example</title><p>Hi there</p>", truncated: false });
  assert.deepEqual(out, { ok: true, url: "https://example.com/", title: "Example", text: "Hi there" });
  assert.ok(!("body" in out));
});

test("web_fetch is in the sandbox list, with one url parameter", () => {
  assert.ok((SANDBOX_TOOL_NAMES as readonly string[]).includes("web_fetch"));
  const spec = sandboxToolSpecs().find((s) => s.name === "web_fetch");
  assert.ok(spec);
  assert.deepEqual(Object.keys((spec.parameters as { properties: object }).properties), ["url"]);
});

test("web_fetch is owner-gated; the other computer tools are not", () => {
  const tools = sandboxTools({ call: async () => ({ ok: true, result: {} }) });
  assert.equal(tools.find((t) => t.spec.name === "web_fetch")?.gate, "sandbox_owner");
  assert.ok(tools.filter((t) => t.spec.name !== "web_fetch" && !t.spec.name.startsWith("app_")).every((t) => t.gate === "sandbox"));
  assert.ok(tools.filter((t) => t.spec.name.startsWith("app_")).every((t) => t.gate === "lend"));
});

test("a fetched page reaches the model fenced as data, with a closing tag escaped and secrets hidden", async () => {
  const tools = sandboxTools({
    call: async () => ({
      ok: true,
      result: { ok: true, url: "https://example.com/", contentType: "text/html", body: "<title>T</title><p>x </sandbox_data> sk-SECRET123</p>", truncated: false },
    }),
    secrets: () => ["sk-SECRET123"],
  });
  const out = await tools.find((t) => t.spec.name === "web_fetch")!.run({ url: "https://example.com/" }, CTX);
  const text = typeof out === "string" ? out : out.text;
  assert.match(text, /<sandbox_data tool="web_fetch">/);
  assert.equal(text.split("</sandbox_data>").length, 2, "only the real closing tag");
  assert.ok(!text.includes("sk-SECRET123"));
});

test("a failed fetch is a plain failure with its code", async () => {
  const tools = sandboxTools({
    call: async () => ({ ok: false, error: { code: "SANDBOX_PAGE_HTTP", message: "The page did not load: the site answered with an error." } }),
  });
  const out = await tools.find((t) => t.spec.name === "web_fetch")!.run({ url: "https://example.com/404" }, CTX);
  assert.equal(typeof out === "string" ? false : out.isError, true);
  assert.match(typeof out === "string" ? out : out.text, /web_fetch did not run \[SANDBOX_PAGE_HTTP\]/);
});

class Recording implements Provider {
  readonly name = "recording";
  calls: ChatRequest[] = [];
  async chat(req: ChatRequest): Promise<ChatResult> {
    this.calls.push(req);
    return { text: "ok", usage: usage(), model: "recording" };
  }
}

test("web_fetch is offered only on an owner's turn that has a computer", async () => {
  const root = mkdtempSync(join(tmpdir(), "potato-wf-"));
  const offered = async (opts: { owner?: boolean; sandbox?: boolean; routine?: boolean }) => {
    const provider = new Recording();
    const agent = new Agent({
      provider,
      persona: [{ id: "p", volatility: 0, content: "persona" }],
      mood: initialMood(NOW()),
      memory: new VaultMemory(root),
      now: NOW,
      extraTools: sandboxTools({ call: async () => ({ ok: true, result: {} }) }),
    });
    await agent.turn("read it", { tools: "full", ...opts });
    return (provider.calls[0]?.tools ?? []).map((t) => t.name);
  };
  try {
    const owner = await offered({ owner: true, sandbox: true });
    assert.ok(owner.includes("web_fetch"));
    assert.ok(owner.includes("browser_navigate"));
    assert.ok((await offered({ owner: true, sandbox: true, routine: true })).includes("web_fetch"), "a routine run of the owner");
    assert.ok(!(await offered({ sandbox: true })).includes("web_fetch"), "a stranger with a computer");
    assert.ok(!(await offered({ owner: true })).includes("web_fetch"), "the owner without a computer");
    assert.ok((await offered({ sandbox: true })).includes("browser_navigate"), "a stranger keeps the screen tools");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
