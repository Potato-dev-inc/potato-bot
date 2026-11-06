import type { ChatRequest, ChatResult, Msg, Provider, ToolCall, Usage } from "../types.ts";
import { estimateTokens } from "./budget.ts";
import { CodedError, codeForStatus } from "./errors.ts";

export interface OpenAIOptions {
  apiKey: string;
  /** e.g. https://api.deepseek.com or https://opencode.ai/inference/go/openai/v1 */
  baseUrl: string;
  model: string;
  name?: string;
  headers?: Record<string, string>;
  /**
   * "auto" sends nothing and lets the server decide (DeepSeek enables thinking
   * by default). Only providers known to accept the field are given a value.
   */
  thinking?: "auto" | "enabled" | "disabled";
  /** low | high | max — DeepSeek maps medium/xhigh/ultra itself */
  reasoningEffort?: string;
}

interface UsageShape {
  prompt_tokens?: number;
  completion_tokens?: number;
  prompt_cache_hit_tokens?: number;
  prompt_cache_miss_tokens?: number;
  prompt_tokens_details?: { cached_tokens?: number };
}

function promptTokensOf(messages: Msg[]): number {
  return messages.reduce((n, m) => n + estimateTokens(m.content), 0);
}

function toWire(msg: Msg): Record<string, unknown> {
  const wire: Record<string, unknown> = { role: msg.role, content: msg.content };
  if (msg.toolCalls && msg.toolCalls.length > 0) {
    wire["tool_calls"] = msg.toolCalls.map((call) => ({
      id: call.id,
      type: "function",
      function: { name: call.name, arguments: call.arguments },
    }));
  }
  if (msg.toolCallId) wire["tool_call_id"] = msg.toolCallId;
  // DeepSeek: with `tools` in the request, prior reasoning must come back
  // verbatim or the API rejects the whole round.
  if (msg.reasoning && msg.role === "assistant") wire["reasoning_content"] = msg.reasoning;
  return wire;
}

/**
 * The messages as the endpoint takes them. A tool message cannot hold a picture, so the pictures of a run of
 * tool messages follow it as one user message (the usual way to show a model a screenshot a tool took).
 */
export function toWireAll(messages: Msg[]): Array<Record<string, unknown>> {
  const out: Array<Record<string, unknown>> = [];
  let pictures: Array<{ mimeType: string; data: string }> = [];
  const flush = (): void => {
    if (pictures.length === 0) return;
    out.push({
      role: "user",
      content: [
        { type: "text", text: "The picture from the tool result above:" },
        ...pictures.map((p) => ({ type: "image_url", image_url: { url: `data:${p.mimeType};base64,${p.data}` } })),
      ],
    });
    pictures = [];
  };
  for (const msg of messages) {
    if (msg.role !== "tool") flush();
    out.push(toWire(msg));
    if (msg.role === "tool" && msg.images) pictures.push(...msg.images);
  }
  flush();
  return out;
}

interface WireToolCall {
  id?: unknown;
  function?: { name?: unknown; arguments?: unknown };
}

function parseToolCalls(raw: unknown): ToolCall[] {
  if (!Array.isArray(raw)) return [];
  const calls: ToolCall[] = [];
  for (const item of raw as WireToolCall[]) {
    const name = typeof item?.function?.name === "string" ? item.function.name : "";
    if (name === "") continue;
    calls.push({
      id: typeof item.id === "string" ? item.id : `call-${calls.length}`,
      name,
      arguments:
        typeof item.function?.arguments === "string" ? item.function.arguments : "{}",
    });
  }
  return calls;
}

function xmlText(value: string): string {
  return value
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&quot;", '"')
    .replaceAll("&apos;", "'")
    .replaceAll("&amp;", "&");
}

/**
 * Some models (DeepSeek's newer chat templates) print a tool call into `content` instead of the OpenAI
 * `tool_calls` field, in a `<||DSML|| calls>` block. This reads that block so such a call still runs, and
 * returns the text without it. A text with no block comes back unchanged with no calls.
 */
export function parseTextToolCalls(raw: string): { text: string; calls: ToolCall[] } {
  const text = raw.replaceAll("\uFF5C", "|");
  const open = text.indexOf("<||DSML|| calls>");
  const close = text.indexOf("</||DSML|| calls>");
  if (open < 0 || close < 0 || close < open) return { text: raw, calls: [] };
  const end = close + "</||DSML|| calls>".length;
  const block = text.slice(open, end);
  const calls: ToolCall[] = [];
  const invoke = /<\|{2}DSML\|{2}\s+invoke name="([^"]+)">([\s\S]*?)<\/\|{2}DSML\|{2}\s+invoke>/g;
  const parameter = /<\|{2}DSML\|{2}\s+parameter name="([^"]+)"(?:\s+string="(true|false)")?>([\s\S]*?)<\/\|{2}DSML\|{2}\s+parameter>/g;
  for (const call of block.matchAll(invoke)) {
    const name = xmlText(call[1] ?? "").trim();
    if (name === "") continue;
    const args: Record<string, unknown> = {};
    for (const p of (call[2] ?? "").matchAll(parameter)) {
      const key = xmlText(p[1] ?? "").trim();
      if (key === "") continue;
      const value = xmlText(p[3] ?? "");
      if (p[2] === "false") {
        try {
          args[key] = JSON.parse(value.trim());
        } catch {
          args[key] = value;
        }
      } else {
        args[key] = value;
      }
    }
    calls.push({ id: `dsml-${calls.length}`, name, arguments: JSON.stringify(args) });
  }
  if (calls.length === 0) return { text: raw, calls: [] };
  return { text: (raw.slice(0, open) + raw.slice(end)).trim(), calls };
}

type Attempt = { ok: true; result: ChatResult } | { ok: false; reason: string; usage: Usage };

/** The room a retry gets after an empty or cut-off reply: four times the budget, between these bounds. */
const MIN_RETRY_TOKENS = 4000;
const MAX_RETRY_TOKENS = 16000;

/** A server fault or an unreadable reply is worth one more try; 4xx (401, 402, 403, 429 too) and a lost network are not. */
function retryable(err: CodedError): boolean {
  if (err.status !== undefined) return err.status >= 500;
  return err.reason === "unreadable reply";
}

function safeFinish(raw: unknown): string {
  return typeof raw === "string" && /^[\w-]{1,24}$/.test(raw) ? raw : "unknown";
}

function addUsage(a: Usage, b: Usage): Usage {
  return {
    promptTokens: a.promptTokens + b.promptTokens,
    completionTokens: a.completionTokens + b.completionTokens,
    cacheHitTokens: a.cacheHitTokens + b.cacheHitTokens,
    cacheMissTokens: a.cacheMissTokens + b.cacheMissTokens,
  };
}

function usageOf(u: UsageShape, messages: Msg[], text: string): Usage {
  const promptTokens = u.prompt_tokens ?? promptTokensOf(messages);
  const reported = u.prompt_cache_hit_tokens ?? u.prompt_tokens_details?.cached_tokens ?? 0;
  const hit = Math.max(0, Math.min(reported, promptTokens));
  const miss = u.prompt_cache_miss_tokens ?? Math.max(0, promptTokens - hit);
  return { promptTokens, completionTokens: u.completion_tokens ?? estimateTokens(text), cacheHitTokens: hit, cacheMissTokens: miss };
}

/** Wait, but end at once when Stop is pressed. */
function pause(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const stopped = (): CodedError => new CodedError("NETWORK", "stopped", undefined, "stopped");
    if (signal?.aborted) return reject(stopped());
    const onAbort = (): void => {
      clearTimeout(timer);
      reject(stopped());
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

/**
 * Any OpenAI-compatible chat completions endpoint: DeepSeek, OpenCode Go,
 * OpenRouter, vLLM, llama.cpp. One implementation, no SDK.
 */
export class OpenAICompatProvider implements Provider {
  readonly name: string;
  private apiKey: string;
  private baseUrl: string;
  private defaultModel: string;
  private extraHeaders: Record<string, string>;
  private thinking: "auto" | "enabled" | "disabled";
  private reasoningEffort: string;

  constructor(opts: OpenAIOptions) {
    this.name = opts.name ?? "openai-compatible";
    this.apiKey = opts.apiKey;
    this.baseUrl = opts.baseUrl.replace(/\/+$/, "");
    this.defaultModel = opts.model;
    this.extraHeaders = opts.headers ?? {};
    this.thinking = opts.thinking ?? "auto";
    this.reasoningEffort = (opts.reasoningEffort ?? "").trim().toLowerCase();
  }

  /** Retry pause for a 5xx or unreadable reply. A field so tests can shorten it. */
  retryDelayMs = 1000;

  async chat(req: ChatRequest): Promise<ChatResult> {
    const model = req.model ?? this.defaultModel;
    const budget = req.maxTokens ?? 400;

    let first: Attempt;
    try {
      first = await this.attempt(req, model, budget, false);
    } catch (err) {
      if (!(err instanceof CodedError) || !retryable(err)) throw err;
      await pause(this.retryDelayMs, req.signal);
      const again = await this.attempt(req, model, budget, false);
      if (again.ok) return again.result;
      throw new CodedError("PROVIDER_DOWN", `${this.name}/${model}: ${again.reason}`, undefined, again.reason);
    }
    if (first.ok) return first.result;
    // Empty or cut off: the budget went on thinking, or the tool call hit the limit. Once more, with room.
    const bigger = Math.min(MAX_RETRY_TOKENS, Math.max(MIN_RETRY_TOKENS, budget * 4));
    const second = await this.attempt(req, model, bigger, true);
    if (second.ok) return { ...second.result, usage: addUsage(first.usage, second.result.usage) };
    throw new CodedError("PROVIDER_DOWN", `${this.name}/${model}: ${second.reason}`, undefined, second.reason);
  }

  private async attempt(req: ChatRequest, model: string, maxTokens: number, retry: boolean): Promise<Attempt> {
    let res: Response;
    try {
      res = await fetch(`${this.baseUrl}/chat/completions`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${this.apiKey}`,
          "User-Agent": "fishd/0.1",
          ...this.extraHeaders,
        },
        body: JSON.stringify({
          model,
          messages: toWireAll(req.messages),
          ...(req.tools && req.tools.length > 0
            ? {
                tools: req.tools.map((tool) => ({
                  type: "function",
                  function: {
                    name: tool.name,
                    description: tool.description,
                    parameters: tool.parameters,
                  },
                })),
              }
            : {}),
          temperature: req.temperature ?? 1.0,
          max_tokens: maxTokens,
          // On the larger retry, thinking goes off, but only where the provider is known to take the field.
          ...(this.thinking === "auto" ? {} : { thinking: { type: retry ? "disabled" : this.thinking } }),
          ...(this.reasoningEffort === "" || this.reasoningEffort === "auto"
            ? {}
            : { reasoning_effort: this.reasoningEffort }),
          stream: false,
        }),
        signal: req.signal,
      });
    } catch {
      // the cause can carry the url or headers; the code is all that leaves
      throw new CodedError("NETWORK", `${this.name}: network error`, undefined, "network error");
    }

    if (!res.ok) {
      // The body is never read: it can echo the request or the account.
      void res.body?.cancel().catch(() => {});
      throw new CodedError(codeForStatus(res.status), `${this.name} ${res.status}`, res.status, `http ${res.status}`);
    }

    let data: {
      choices?: Array<{
        message?: { content?: string; reasoning_content?: string; tool_calls?: unknown };
        finish_reason?: string;
      }>;
      usage?: UsageShape;
    };
    try {
      data = (await res.json()) as typeof data;
    } catch {
      throw new CodedError("PROVIDER_DOWN", `${this.name}: unreadable reply`, undefined, "unreadable reply");
    }

    const choice = data.choices?.[0];
    let text = choice?.message?.content ?? "";
    let toolCalls = parseToolCalls(choice?.message?.tool_calls);
    // Some DeepSeek builds print the call in the text; read it so the turn still works.
    if (text.includes("DSML")) {
      const printed = parseTextToolCalls(text);
      if (printed.calls.length > 0) {
        if (toolCalls.length === 0) toolCalls = printed.calls;
        text = printed.text;
      }
    }
    const reasoning = choice?.message?.reasoning_content ?? "";

    const finishReason = safeFinish(choice?.finish_reason);
    const u = data.usage ?? {};
    const empty = text.trim() === "" && toolCalls.length === 0;
    // A tool call stopped by the limit has cut-off arguments: it is never passed on.
    const cutOff = toolCalls.length > 0 && finishReason === "length";
    if (empty || cutOff) {
      const reason = empty ? `empty reply finish=${finishReason}` : "tool call cut off finish=length";
      return { ok: false, reason, usage: usageOf(u, req.messages, text) };
    }
    const usage = usageOf(u, req.messages, text);

    return {
      ok: true,
      result: {
        text,
        usage,
        model,
        toolCalls: toolCalls.length > 0 ? toolCalls : undefined,
        reasoning: reasoning === "" ? undefined : reasoning,
      },
    };
  }
}
