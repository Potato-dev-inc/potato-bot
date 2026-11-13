import type { ChatRequest, ChatResult, Msg, Provider, ToolCall, Usage } from "../types.ts";
import { estimateTokens } from "./budget.ts";
import { modelSeesImages } from "./models.ts";
import { CONTEXT_TOO_LONG_REASON, CodedError, codeForStatus } from "./errors.ts";

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

function toWire(msg: Msg, userImages: boolean): Record<string, unknown> {
  const wire: Record<string, unknown> = { role: msg.role, content: msg.content };
  // The pictures a person attached ride on their own message, for a model that can see.
  if (userImages && msg.role === "user" && msg.images && msg.images.length > 0) {
    wire["content"] = [
      { type: "text", text: msg.content },
      ...msg.images.map((p) => ({ type: "image_url", image_url: { url: `data:${p.mimeType};base64,${p.data}` } })),
    ];
  }
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
export function toWireAll(messages: Msg[], userImages: boolean = true): Array<Record<string, unknown>> {
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
    out.push(toWire(msg, userImages));
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
 * DSML markers: one or two pipes on each side of DSML, ASCII `|` or fullwidth `\uFF5C` (the fullwidth ones are
 * turned into `|` before matching), with optional spaces around and inside the bars. Written with
 * String.raw so the backslashes stay literal in the regex source.
 */
const DSML_BAR = String.raw`\|{1,2}\s*DSML\s*\|{1,2}`;
const DSML_BLOCK_NAME = String.raw`(?:function_calls|calls)`;
/** Group 1: the body of a calls block. Group 2: its closing tag, or "" when the reply ends before it. */
const DSML_BLOCK = new RegExp(
  String.raw`<\s*${DSML_BAR}\s*${DSML_BLOCK_NAME}\s*>([\s\S]*?)(<\/\s*${DSML_BAR}\s*${DSML_BLOCK_NAME}\s*>|$)`,
  "g",
);
const DSML_INVOKE = new RegExp(
  String.raw`<\s*${DSML_BAR}\s*invoke\s+name\s*=\s*"([^"]+)"\s*>([\s\S]*?)<\/\s*${DSML_BAR}\s*invoke\s*>`,
  "g",
);
const DSML_PARAMETER = new RegExp(
  String.raw`<\s*${DSML_BAR}\s*parameter\s+name\s*=\s*"([^"]+)"(?:\s+string\s*=\s*"(true|false)")?\s*>([\s\S]*?)<\/\s*${DSML_BAR}\s*parameter\s*>`,
  "g",
);

/** Reads the invoke elements of one closed calls block into `calls`, in order. */
function readDsmlCalls(body: string, calls: ToolCall[]): void {
  for (const call of body.matchAll(DSML_INVOKE)) {
    const name = xmlText(call[1] ?? "").trim();
    if (name === "") continue;
    const args: Record<string, unknown> = {};
    for (const p of (call[2] ?? "").matchAll(DSML_PARAMETER)) {
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
}

/**
 * Some models (DeepSeek's newer chat templates) print a tool call into `content` instead of the OpenAI
 * `tool_calls` field, in a DSML `calls` (or `function_calls`) block. This reads every closed block so such
 * calls still run, and returns the text with the blocks taken out. A block the reply cut off is taken out
 * too, but none of its calls run. Text with no block, or only closed blocks with no calls, comes back unchanged.
 */
export function parseTextToolCalls(raw: string): { text: string; calls: ToolCall[] } {
  const text = raw.replaceAll("\uFF5C", "|");
  const calls: ToolCall[] = [];
  let visible = "";
  let last = 0;
  let cutOff = false;
  for (const block of text.matchAll(DSML_BLOCK)) {
    const at = block.index ?? 0;
    visible += raw.slice(last, at);
    last = at + block[0].length;
    if ((block[2] ?? "") === "") {
      cutOff = true;
      continue;
    }
    readDsmlCalls(block[1] ?? "", calls);
  }
  if (calls.length === 0 && !cutOff) return { text: raw, calls: [] };
  visible += raw.slice(last);
  return { text: visible.trim(), calls };
}

type Attempt = { ok: true; result: ChatResult } | { ok: false; reason: string; usage: Usage; cut?: ChatResult };

/** The room a retry gets after an empty or cut-off reply: four times the budget, between these bounds. */
const MIN_RETRY_TOKENS = 4000;
const MAX_RETRY_TOKENS = 16000;

/** Whether an error reply says the prompt does not fit the model's window. Reads a little of the body; keeps none of it. */
async function bodyMeansTooLong(res: Response): Promise<boolean> {
  try {
    const text = (await res.text()).slice(0, 4000);
    return /context[_ ]length|context window|maximum context|too many tokens|prompt is too long|input.{0,20}too long|exceeds? the (?:model|maximum)|reduce the length/i.test(text);
  } catch {
    return false;
  }
}

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
  get modelName(): string {
    return this.defaultModel;
  }
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
    // Cut off even with room: the loop hears of it (cutOff) and tells the model to write smaller pieces. Nothing runs.
    if (second.cut) return { ...second.cut, usage: addUsage(first.usage, second.cut.usage) };
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
          messages: toWireAll(req.messages, modelSeesImages(model)),
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
      // Only a 400/413 body is looked at, for the one question "was the request too long?"; none of it is kept or passed on.
      if ((res.status === 400 || res.status === 413) && (await bodyMeansTooLong(res))) {
        throw new CodedError(codeForStatus(res.status), `${this.name} ${res.status}`, res.status, CONTEXT_TOO_LONG_REASON);
      }
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
      // Also takes out a block the reply cut off (no calls from it), so its markup is never shown.
      const printed = parseTextToolCalls(text);
      if (toolCalls.length === 0) toolCalls = printed.calls;
      text = printed.text;
    }
    const reasoning = choice?.message?.reasoning_content ?? "";

    const finishReason = safeFinish(choice?.finish_reason);
    const u = data.usage ?? {};
    const empty = text.trim() === "" && toolCalls.length === 0;
    // A tool call stopped by the limit has cut-off arguments: it is never passed on.
    const cutOff = toolCalls.length > 0 && finishReason === "length";
    if (empty || cutOff) {
      const reason = empty ? `empty reply finish=${finishReason}` : "tool call cut off finish=length";
      const spent = usageOf(u, req.messages, text);
      const cut: ChatResult | undefined = cutOff
        ? { text, usage: spent, model, toolCalls, cutOff: true, ...(reasoning === "" ? {} : { reasoning }) }
        : undefined;
      return { ok: false, reason, usage: spent, ...(cut ? { cut } : {}) };
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
