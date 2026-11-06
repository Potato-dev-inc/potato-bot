import type { ExtraTool } from "./loop.ts";

/**
 * message_user: the bot sends its owner a message while it is still working (a progress note on a long job).
 * Owner's own turns only (`gate: "owner"`). It goes to the owner's own conversation through `send`, the same
 * channel as the turn's reply. At most MESSAGE_USER_TURN_CAP per turn, so a long job cannot flood the chat.
 */
export const MESSAGE_USER_NAME = "message_user";
export const MESSAGE_USER_TURN_CAP = 3;
const MESSAGE_CHARS = 500;

export function messageUserTool(send: (text: string) => boolean): ExtraTool {
  let sent = { turn: -1, count: 0 };
  return {
    gate: "owner",
    spec: {
      name: MESSAGE_USER_NAME,
      description: `Send your owner a short message now, while you are still working, for example a progress note on a long job. At most ${MESSAGE_USER_TURN_CAP} per reply. Your final answer is still sent as usual; do not repeat the same text in it.`,
      parameters: {
        type: "object",
        properties: {
          text: { type: "string", description: `The message, ${MESSAGE_CHARS} characters at most` },
        },
        required: ["text"],
      },
    },
    run(args, ctx) {
      if (sent.turn !== ctx.turn) sent = { turn: ctx.turn, count: 0 };
      if (sent.count >= MESSAGE_USER_TURN_CAP) {
        return "You already sent the owner the most messages for this reply. Put the rest in your answer.";
      }
      const text = (typeof args["text"] === "string" ? args["text"] : "").trim().slice(0, MESSAGE_CHARS);
      if (text === "") return "Nothing to send.";
      if (!send(text)) return "The message could not be sent right now.";
      sent.count++;
      return "Sent.";
    },
  };
}
