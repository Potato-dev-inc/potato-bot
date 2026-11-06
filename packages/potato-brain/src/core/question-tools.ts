import type { ExtraTool } from "./loop.ts";
import type { SandboxReply } from "./sandbox-tools.ts";

/**
 * ask_user: the bot asks its owner a question it cannot go on without. A turn cannot wait for an answer inside
 * the model loop, so the turn ends here with the question as its reply; the owner's next message is the answer,
 * and it arrives as a new turn that already holds this question in the history. Owner's own chats only
 * (`gate: "owner_person"`): a routine's run has no person to answer.
 *
 * In the app's private chat, with choices, the question becomes a card with buttons (the app keeps it, so it
 * survives a restart); the turn still ends, and the button press arrives as the next turn. Everywhere else
 * (Telegram, no choices, an app that does not know the card) it stays plain text.
 *
 * request_approval: "may I do this?" as a card with Allow once / Allow for this job / Deny, or, with `choices`, as
 * a question card. Only the app can answer a card, with a button; nothing the model or a chat writes can.
 */
export const ASK_USER_NAME = "ask_user";
export const REQUEST_APPROVAL_NAME = "request_approval";

/** The turn's last line when a card was made: the card itself is the question. */
export const CARD_WAIT_TEXT = "Waiting for your answer on the card above.";

const QUESTION_CHARS = 400;
const CHOICE_CHARS = 60;
const CHOICES_MAX = 4;
const ACTION_CHARS = 300;
const WHY_CHARS = 400;

export type CardToolName = typeof ASK_USER_NAME | typeof REQUEST_APPROVAL_NAME;
type CardCall = (tool: CardToolName, args: Record<string, unknown>) => Promise<SandboxReply>;

export interface AskUserOptions {
  /** sends the question to the app as a card; absent: plain text only */
  call?: CardCall;
  /** only the app's private chat has cards; a Telegram chat or a group never calls the app */
  inAppDm?: boolean;
}

export function askUserTool(opts: AskUserOptions = {}): ExtraTool {
  return {
    gate: "owner_person",
    spec: {
      name: ASK_USER_NAME,
      description:
        "Ask your person a question you cannot go on without. Your turn ends with the question, and their answer comes as their next message. Only ask when you truly need the answer. Give up to 4 short choices when the answer is one of a few: in the app they are buttons.",
      parameters: {
        type: "object",
        properties: {
          question: { type: "string", description: `The question in plain words, ${QUESTION_CHARS} characters at most` },
          choices: {
            type: "array",
            items: { type: "string" },
            maxItems: CHOICES_MAX,
            description: `Optional short answers to offer, ${CHOICE_CHARS} characters each`,
          },
        },
        required: ["question"],
      },
    },
    run(args) {
      const question = str(args["question"]).trim().slice(0, QUESTION_CHARS);
      if (question === "") return "Write the question first.";
      const choices = cleanChoices(args["choices"]);
      const asText = (): { text: string; endTurn: true } => ({
        text: choices.length > 0 ? `${question}\n(${choices.join(" / ")})` : question,
        endTurn: true,
      });
      if (choices.length > 0 && opts.call && opts.inAppDm) {
        // a card if the app makes one; the plain question if not
        return makeCard(opts.call, ASK_USER_NAME, { question, choices }).then((card) => (card === "made" ? { text: CARD_WAIT_TEXT, endTurn: true } : asText()));
      }
      return asText();
    },
  };
}

export function requestApprovalTool(opts: { call: CardCall }): ExtraTool {
  return {
    gate: "owner_person",
    spec: {
      name: REQUEST_APPROVAL_NAME,
      description:
        "Ask your owner to approve something before you do it (a deploy, a purchase, sending something out, deleting things). The app shows a card; your turn ends; the answer (Allow once, Allow for this job, Deny, or a choice) comes as your next message. Never say something was approved until that answer arrives. Give up to 4 short choices to ask a question with buttons instead.",
      parameters: {
        type: "object",
        properties: {
          action: { type: "string", description: `What you want to do, in plain words (${ACTION_CHARS} characters at most)` },
          why: { type: "string", description: `Why you need to, one or two sentences (${WHY_CHARS} characters at most)` },
          choices: {
            type: "array",
            items: { type: "string" },
            maxItems: CHOICES_MAX,
            description: `Optional short answers; with them the card is a question, not an approval (${CHOICE_CHARS} characters each)`,
          },
        },
        required: ["action", "why"],
      },
    },
    async run(args) {
      const action = str(args["action"]).trim().slice(0, ACTION_CHARS);
      if (action === "") return { text: "Write what you want to do first.", isError: true };
      const why = str(args["why"]).trim().slice(0, WHY_CHARS);
      const choices = cleanChoices(args["choices"]);
      const sent: Record<string, unknown> = { action, why, ...(choices.length > 0 ? { choices } : {}) };
      const card = await makeCard(opts.call, REQUEST_APPROVAL_NAME, sent);
      if (card === "made") return { text: CARD_WAIT_TEXT, endTurn: true };
      return { text: `request_approval did not run: ${card.message}`, isError: true };
    },
  };
}

/** Sends the card to the app. Never throws: any failure is a plain message. */
async function makeCard(call: CardCall, tool: CardToolName, args: Record<string, unknown>): Promise<"made" | { message: string }> {
  let reply: SandboxReply;
  try {
    reply = await call(tool, args);
  } catch {
    return { message: "The app did not answer." };
  }
  if (!reply.ok) return { message: reply.error.message };
  return reply.result["card"] === true ? "made" : { message: "The app made no card." };
}

function cleanChoices(raw: unknown): string[] {
  return Array.isArray(raw)
    ? raw
        .filter((c): c is string => typeof c === "string")
        .map((c) => c.trim().slice(0, CHOICE_CHARS))
        .filter((c) => c !== "")
        .slice(0, CHOICES_MAX)
    : [];
}

function str(v: unknown): string {
  return typeof v === "string" ? v : "";
}
