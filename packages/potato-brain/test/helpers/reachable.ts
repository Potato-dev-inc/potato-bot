import type { ChatRequest } from "../../src/types.ts";

/**
 * The tool names a turn could use: the ones in the request, and the ones find_tool can reach (use_tool runs those). A long tool
 * list is cut to the core tools plus find_tool and use_tool (src/core/tool-bridge.ts); find_tool's own description names every
 * tool left out, and that set is built by the same gates as the list, so "reachable" is exactly "the turn may use it".
 */
export function reachable(request: ChatRequest | undefined): string[] {
  const tools = request?.tools ?? [];
  const names = tools.map((t) => t.name);
  const finder = tools.find((t) => t.name === "find_tool");
  const listed = finder ? /Tools you can find: (.*)\.$/.exec(finder.description)?.[1] : undefined;
  return listed ? [...names, ...listed.split(", ")] : names;
}
