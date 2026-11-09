import type { ToolSpec } from "../types.ts";

/**
 * Find-tool and use-tool: when a turn would send a long tool list, the model gets the tools it uses all the time plus two
 * small tools, and reaches the rest by name. Every tool in the list went out on every turn, about 30 000 characters of
 * JSON before the person's words; most turns use a handful.
 *
 * What does NOT change: the set of tools a turn may reach is computed once, in `Agent.tools()`, by the same gates as the
 * tools it sends (sandbox, owner, owner-and-not-a-routine, room, memory level). `use_tool` can only name a tool in that
 * set. It then runs the tool exactly as if the model had called it by its own name: the same `run`, the same host call,
 * so the host's checks (When to ask, room rules, the switches) see the real tool name and never "use_tool".
 * Written for Potato; no code copied.
 */
export const FIND_TOOL_NAME = "find_tool";
export const USE_TOOL_NAME = "use_tool";

/** A turn with at most this many tools sends them all, as before. */
export const BRIDGE_MIN_TOOLS = 28;
/** The bridge is worth its two tools only when it hides at least this many. */
export const BRIDGE_MIN_HIDDEN = 5;

/** Extra tools that stay in every list: the computer's everyday tools, the questions and cards, the to-do the loop relies on. */
export const CORE_EXTRA_TOOLS: ReadonlySet<string> = new Set([
  "ask_helper",
  "run_terminal",
  "read_file",
  "write_file",
  "edit_file",
  "list_files",
  "web_fetch",
  "computer_observe",
  "computer_act",
  "browser_navigate",
  "browser_snapshot",
  "browser_act",
  "mcp_list_tools",
  "mcp_call_tool",
  "ask_user",
  "request_approval",
  "message_user",
  "hand_off",
  "message_bot",
  "job_log",
  "job_done",
  "run_subagent",
  "skill_read",
  // The owner's lent Windows apps: offered only on a turn where the app said `lend`, so never a cost otherwise. A bot that only
  // finds them through find_tool never learns it was lent an app, and tells the person it has no such tools.
  "app_observe",
  "app_act",
]);

const FIND_MAX = 6;
const NAMES_IN_SPEC = 80;

export function findToolSpec(names: string[]): ToolSpec {
  return {
    name: FIND_TOOL_NAME,
    description:
      "Look up a tool that is not in your list. Say what you want to do in a few words; you get the matching tools with what each one does and the arguments it takes. " +
      "Then call it with use_tool. Tools you can find: " +
      names.slice(0, NAMES_IN_SPEC).join(", ") +
      ".",
    parameters: {
      type: "object",
      properties: { query: { type: "string", description: "What you want to do, or part of a tool's name (empty lists them all)" } },
    },
  };
}

export function useToolSpec(): ToolSpec {
  return {
    name: USE_TOOL_NAME,
    description:
      "Run a tool that find_tool showed you. Give its name and its arguments as find_tool listed them. It follows exactly the same rules as if it were in your list: anything that asks your owner still asks.",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string", description: "The tool's name, exactly as find_tool shows it" },
        arguments: { type: "object", description: "The tool's arguments" },
      },
      required: ["name"],
    },
  };
}

function words(text: string): string[] {
  return text.toLowerCase().split(/[^a-z0-9一-鿿]+/).filter((w) => w.length > 1);
}

/** The tools whose name or description match the query, best first. An empty query lists them all. Plain text. */
export function searchTools(query: string, hidden: ToolSpec[]): string {
  const q = words(query);
  const scored = hidden.map((spec) => {
    const name = spec.name.toLowerCase();
    const hay = words(`${spec.name.replace(/_/g, " ")} ${spec.description}`);
    let score = 0;
    for (const w of q) {
      if (name === w) score += 6;
      else if (name.includes(w)) score += 4;
      if (hay.includes(w)) score += 1;
    }
    return { spec, score };
  });
  const picked = q.length === 0 ? scored : scored.filter((s) => s.score > 0).sort((a, b) => b.score - a.score);
  if (picked.length === 0) {
    return `No tool matches. Tools you can find: ${hidden.map((s) => s.name).join(", ")}.`;
  }
  const shown = q.length === 0 ? picked : picked.slice(0, FIND_MAX);
  if (q.length === 0) {
    return shown.map(({ spec }) => `${spec.name}: ${spec.description.split(/(?<=[.!?。])\s/)[0]?.slice(0, 140) ?? ""}`).join("\n");
  }
  return shown
    .map(({ spec }) => `## ${spec.name}\n${spec.description}\narguments: ${JSON.stringify(spec.parameters)}`)
    .join("\n\n");
}
