/**
 * Live steering. A person's message that arrives while a turn is running is not made to wait for the whole turn
 * (a job turn can run for dozens of tool rounds). The turn reads it between tool rounds, adds it to what the model
 * sees, and the model answers it at once with message_user and goes on. It is the person's own text: the host only
 * hands it over for the owner's private chat, never for a routine, a room or a stranger.
 */

export interface SteerItem {
  /** the host's id for the message (the host is told it was picked up) */
  id: string;
  text: string;
}

export const STEER_TURN_NOTE =
  "The person may write to you while you work. Such a message reaches you between tool rounds, marked as written while you were working. " +
  "Answer it right away with message_user, then go on with the work. A question about how it is going gets one to three plain lines of true state taken from your to-do list and the job log (what is done, what you are on now, what is left); never make up progress. " +
  "If the message changes the goal or asks for something different, change your plan and your to-do list to match. " +
  "Words like \"stop\" are not special to you: the Stop button is the real stop.";

/** The text the model sees for the messages picked up between two rounds. */
export function steerMessage(items: readonly SteerItem[]): string {
  const lines = items.map((item) => item.text.trim()).filter((text) => text !== "");
  const head =
    lines.length === 1
      ? "[The person wrote this while you were working. Answer it now with message_user, then carry on unless it changes the job.]"
      : "[The person wrote these while you were working, oldest first. Answer them now with message_user, then carry on unless they change the job.]";
  return `${head}\n${lines.join("\n")}`;
}
