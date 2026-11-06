// Where a "My TUI agent" bot talks: the companion's terminal, an ACP chat it started, or a running agent pane picked
// from the full New agent flow. The choice is per bot and kept on this computer; the bot thread and the Advanced
// panel both read it, and a small window event keeps them in step.

export const AGENT_CHOICE_EVENT = 'pm.potato.agent.changed';

const choiceKey = (botId: string): string => `pm.potato.agent.${botId}`;
const acpKey = (botId: string, agentId: string): string => `pm.potato.acp.${botId}.${agentId}`;

function read(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string | null): void {
  try {
    if (value === null) window.localStorage.removeItem(key);
    else window.localStorage.setItem(key, value);
  } catch {
    // Blocked storage: the choice simply is not kept.
  }
}

/** `tui` (the companion's own terminal), `acp:<agentId>`, or `pane:<paneId>`. */
export function readAgentChoice(botId: string): string {
  return read(choiceKey(botId)) ?? 'tui';
}

export function writeAgentChoice(botId: string, value: string): void {
  write(choiceKey(botId), value);
}

export function acpPaneOf(choice: string): string | null {
  return choice.startsWith('acp:') ? choice.slice(4) : null;
}

export function boundPaneOf(choice: string): string | null {
  return choice.startsWith('pane:') ? choice.slice(5) : null;
}

export function readAcpPane(botId: string, agentId: string): string | null {
  return read(acpKey(botId, agentId));
}

export function writeAcpPane(botId: string, agentId: string, paneId: string | null): void {
  write(acpKey(botId, agentId), paneId);
}

/** The bot thread and the Advanced panel both listen for this. */
export function announceAgentChoice(botId: string): void {
  try {
    window.dispatchEvent(new CustomEvent(AGENT_CHOICE_EVENT, { detail: { botId } }));
  } catch {
    // No window (tests with no DOM): nothing to tell.
  }
}

/** The plain message sent to a TUI agent that did not learn about the sandbox from its brief. */
export const SANDBOX_NOTICE =
  "Puppet Master instruction (no reply needed): this computer has a Linux sandbox you can use. " +
  "Run it from this Windows shell with: wsl.exe -d pm-sbx-main -u pm --exec /usr/local/bin/pm-sbx-cli run \"<command>\". " +
  "The same CLI drives its desktop: status, page (text of the front Chrome tab), shot /tmp/shot.png, " +
  "click X Y, move X Y, drag X Y TO_X TO_Y, type \"text\", key ctrl+l, scroll X Y down 3, " +
  "and open chrome / terminal / files / editor / paint. " +
  "If the person set up Vercel tools in the sandbox, the Vercel CLI and mcp-remote are in ~/.local/bin there: " +
  "pm-sbx-cli run \"vercel whoami\", pm-sbx-cli run \"vercel deploy --yes\", or pm-sbx-cli run \"mcp-remote https://mcp.vercel.com\" (sign in inside the sandbox's Chrome). " +
  "If pm-sbx-cli is missing, stop and start the sandbox in Settings > Potato Bot > Sandbox; Puppet Master installs the current tools when it starts.";
