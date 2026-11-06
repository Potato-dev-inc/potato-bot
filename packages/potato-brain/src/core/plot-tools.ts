import type { ExtraTool } from "./loop.ts";
import type { SandboxReply } from "./sandbox-tools.ts";

/**
 * `render_plot`: a bar, line or pie chart in the owner's private chat. The bot passes the numbers; the app keeps them
 * and draws the chart itself (nothing is rendered here, no markup is sent). The same limits as the app
 * (`service/chart.rs`): too much is refused with one plain sentence, never cut.
 *
 * When to ask: no card at any level. It reads nothing, runs nothing and sends nothing out; it only shows the bot's own
 * chart in the chat. Owner turns only (`gate: "owner"`); not offered in a group (see ROOM_OFF_TOOLS).
 * Written for this app; no code copied.
 */
export const PLOT_TOOL_NAME = "render_plot";

export const PLOT_MAX_POINTS = 200;
export const PLOT_MAX_PIE_POINTS = 12;
export const PLOT_TITLE_MAX = 80;
export const PLOT_LABEL_MAX = 40;
export const PLOT_VALUE_MAX = 1e12;

export const PLOT_SPEC = {
  name: PLOT_TOOL_NAME,
  description:
    "Show a chart in the chat: a bar, line or pie chart of numbers you give. Use it when a picture says it better than a list, " +
    "for example a few totals, a trend over time or shares of a whole. Then say in a line or two what it shows; do not repeat all the numbers.",
  parameters: {
    type: "object",
    properties: {
      type: { type: "string", enum: ["bar", "line", "pie"], description: "bar compares amounts, line shows change in order, pie shows shares of a whole (no negative values)" },
      title: { type: "string", description: `A short title, ${PLOT_TITLE_MAX} characters at most` },
      points: {
        type: "array",
        description: `${PLOT_MAX_POINTS} points at most (${PLOT_MAX_PIE_POINTS} for a pie), in the order to draw them`,
        items: {
          type: "object",
          properties: {
            label: { type: "string", description: `Plain text, ${PLOT_LABEL_MAX} characters at most` },
            value: { type: "number", description: "A plain number" },
          },
          required: ["label", "value"],
        },
      },
    },
    required: ["type", "points"],
  },
};

/** One plain sentence for the model, or null when the call may go to the app. */
export function checkPlotArgs(args: Record<string, unknown>): string | null {
  const type = args["type"];
  if (type !== "bar" && type !== "line" && type !== "pie") return 'Say which chart: type must be "bar", "line" or "pie".';
  const title = args["title"];
  if (title !== undefined && (typeof title !== "string" || [...title].length > PLOT_TITLE_MAX)) {
    return `Use a title of ${PLOT_TITLE_MAX} characters or fewer.`;
  }
  const points = args["points"];
  if (!Array.isArray(points)) return 'Give points: a list of {"label": text, "value": number}.';
  const most = type === "pie" ? PLOT_MAX_PIE_POINTS : PLOT_MAX_POINTS;
  if (points.length < 1 || points.length > most) return `Give between 1 and ${most} points for this chart.`;
  if (type === "line" && points.length < 2) return "A line needs at least 2 points.";
  let sum = 0;
  for (let i = 0; i < points.length; i++) {
    const n = i + 1;
    const p = points[i];
    const rec = p !== null && typeof p === "object" && !Array.isArray(p) ? (p as Record<string, unknown>) : {};
    const label = rec["label"];
    if (typeof label !== "string" || label.trim() === "") return `Point ${n} needs a label.`;
    if ([...label].length > PLOT_LABEL_MAX) return `Point ${n}: use a label of ${PLOT_LABEL_MAX} characters or fewer.`;
    const value = rec["value"];
    if (typeof value !== "number" || !Number.isFinite(value)) return `Point ${n}: value must be a number.`;
    if (Math.abs(value) > PLOT_VALUE_MAX) return `Point ${n}: value must be a number below a trillion.`;
    if (type === "pie" && value < 0) return `Point ${n}: a pie cannot have a negative value.`;
    sum += value;
  }
  if (type === "pie" && sum <= 0) return "A pie needs at least one value above zero.";
  return null;
}

/** What the model reads back. */
export function formatPlotReply(reply: SandboxReply): { text: string; isError?: true } {
  if (!reply.ok) return { text: `render_plot did not run [${reply.error.code}]: ${reply.error.message}`, isError: true };
  const note = typeof reply.result["note"] === "string" ? reply.result["note"] : "The chart is now in the chat.";
  return { text: note };
}

/** The tool for one conversation. Only the checked, plain arguments go to the app. */
export function plotTool(opts: { call: (tool: typeof PLOT_TOOL_NAME, args: Record<string, unknown>) => Promise<SandboxReply> }): ExtraTool {
  return {
    spec: { ...PLOT_SPEC },
    gate: "owner",
    async run(args) {
      const bad = checkPlotArgs(args);
      if (bad) return { text: bad, isError: true };
      let reply: SandboxReply;
      try {
        reply = await opts.call(PLOT_TOOL_NAME, args);
      } catch {
        reply = { ok: false, error: { code: "PLOT_FAILED", message: "The chart could not be shown." } };
      }
      return formatPlotReply(reply);
    },
  };
}
