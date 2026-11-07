// The Go plan through a local fake server: the saved key and chosen model reach the plan's chat endpoint exactly as the
// plan's docs describe (Bearer key, a session header, the model id in the body), and a refused key never appears in the
// error the rest of the app sees. The key here is a made-up one; nothing leaves this machine.
import { test, afterEach } from "node:test";
import assert from "node:assert/strict";
import { createServer, type IncomingMessage, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { makeProvider } from "../src/core/provider.ts";

const FAKE = "oc-fake-go-key-0123456789abcdef";
const saved = { ...process.env };
let server: Server | undefined;

afterEach(async () => {
  process.env = { ...saved };
  await new Promise<void>((done) => (server ? server.close(() => done()) : done()));
  server = undefined;
});

async function serve(status: number): Promise<{ base: string; seen: Array<{ url: string; headers: IncomingMessage["headers"]; body: Record<string, unknown> }> }> {
  const seen: Array<{ url: string; headers: IncomingMessage["headers"]; body: Record<string, unknown> }> = [];
  server = createServer((req, res) => {
    let raw = "";
    req.on("data", (c: Buffer) => (raw += c.toString()));
    req.on("end", () => {
      seen.push({ url: req.url ?? "", headers: req.headers, body: JSON.parse(raw || "{}") as Record<string, unknown> });
      res.writeHead(status, { "Content-Type": "application/json" });
      res.end(
        status === 200
          ? JSON.stringify({ choices: [{ message: { content: "hi" }, finish_reason: "stop" }], usage: { prompt_tokens: 3, completion_tokens: 1 } })
          : JSON.stringify({ error: `bad key ${FAKE}` }),
      );
    });
  });
  await new Promise<void>((done) => server!.listen(0, "127.0.0.1", done));
  return { base: `http://127.0.0.1:${(server.address() as AddressInfo).port}/inference/go/openai/v1`, seen };
}

function useGo(base: string, model: string): void {
  process.env = { PATH: process.env.PATH ?? "", FISHD_PROVIDER: "opencode", OPENCODE_API_KEY: FAKE, OPENCODE_BASE_URL: base, FISHD_MODEL: model };
}

test("the Go provider sends the key as a Bearer header with a session header, to the chat endpoint, with the chosen model", async () => {
  const { base, seen } = await serve(200);
  useGo(base, "glm-5.3");
  const res = await makeProvider().chat({ messages: [{ role: "user", content: "hello" }], maxTokens: 50 });
  assert.equal(res.text, "hi");
  assert.equal(seen.length, 1);
  assert.equal(seen[0]!.url, "/inference/go/openai/v1/chat/completions");
  assert.equal(seen[0]!.headers["authorization"], `Bearer ${FAKE}`);
  assert.match(String(seen[0]!.headers["x-opencode-session"]), /^[0-9a-f-]{36}$/);
  assert.equal(seen[0]!.body["model"], "glm-5.3");
  assert.ok(!JSON.stringify(seen[0]!.body).includes(FAKE), "the key is only in the header, never in the request body the model sees");
});

test("a key the plan refuses is reported as a status, with nothing from the server or the key in the error", async () => {
  const { base } = await serve(401);
  useGo(base, "kimi-k3");
  await assert.rejects(
    makeProvider().chat({ messages: [{ role: "user", content: "hello" }], maxTokens: 50 }),
    (error: unknown) => {
      const text = `${String(error)} ${JSON.stringify(error, Object.getOwnPropertyNames(error as object))}`;
      assert.ok(!text.includes(FAKE), text);
      assert.ok(!text.includes("bad key"), text);
      return true;
    },
  );
});
