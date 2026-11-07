// The owner's own agents for a bot ("Through my agents"): the calls behind the Advanced tab's add, approve, pick and remove.
// The names mirror src-tauri/src/commands/potato_bot_settings.rs and potato/custom_agents.rs. Only the owner, on this
// computer, reaches these; the bot has no tool for them.
import { invoke } from '@tauri-apps/api/core';
import type { AgentSecretName } from './secrets';

/** What the owner reads before approving a pasted definition. */
export interface AgentSummary {
  id: string;
  label: string;
  description: string;
  /** The command line, with `<your message>` where the task goes. */
  command: string;
  installs: string;
  envNames: string[];
  /** The names it asks the owner for. The values are typed after approval and kept in the credential store. */
  secretNames: string[];
  fingerprint: string;
  notes: string[];
}

/** One value an agent asks for: the name it reads, and the credential-store name the Settings field sets. */
export interface AgentSecret {
  env: string;
  key: AgentSecretName;
}

export interface AgentRow {
  id: string;
  label: string;
  description: string;
  command: string;
  fingerprint: string;
  approvedAt: number;
  secrets: AgentSecret[];
}

export interface AgentsView {
  agents: AgentRow[];
  brainAgent: string | null;
  ownAccountNote: string;
  decidesNote: string;
  /** The sandbox is on and set up, so an agent turn can run. */
  sandboxReady: boolean;
}

export interface AgentsTransport {
  get(botId: string): Promise<AgentsView>;
  /** Checks a pasted definition. Rejects with `POTATO_AGENT_INVALID: Line N: ...`. Saves nothing. */
  check(definition: string): Promise<AgentSummary>;
  /** The owner approved the summary they read (its fingerprint). */
  add(botId: string, definition: string, fingerprint: string): Promise<AgentsView>;
  remove(botId: string, agentId: string): Promise<AgentsView>;
  /** `null`: no agent. */
  pick(botId: string, agentId: string | null): Promise<AgentsView>;
}

export const tauriAgentsTransport: AgentsTransport = {
  get: (botId) => invoke<AgentsView>('potato_agents_get', { botId }),
  check: (definition) => invoke<AgentSummary>('potato_agent_check', { definition }),
  add: (botId, definition, fingerprint) => invoke<AgentsView>('potato_agent_add', { botId, definition, fingerprint }),
  remove: (botId, agentId) => invoke<AgentsView>('potato_agent_remove', { botId, agentId }),
  pick: (botId, agentId) => invoke<AgentsView>('potato_agent_pick', { botId, agentId }),
};

export const AGENTS_COPY = {
  title: 'Agents in the sandbox',
  line: 'Your own agent can be this bot’s brain. It runs only in the sandbox, never on this PC, and only for your own messages in your private chat.',
  none: 'No agents added yet.',
  add: 'Add an agent',
  paste: 'Paste a definition.toml',
  check: 'Check',
  approve: 'Approve and add',
  use: 'Use as brain',
  inUse: 'In use',
  remove: 'Remove',
  secretsLine: 'Values this agent asks for. Each is kept in your keychain and given only to this agent.',
  asks: 'Asks you for',
  noSandbox: 'Agents run only in the sandbox, and it is not switched on or not set up. Set it up in Settings > Potato Bot first.',
};

/** The text a failed command shows: the part after `CODE: `. */
export function agentErrorText(cause: unknown, fallback: string): string {
  const raw = cause instanceof Error ? cause.message : typeof cause === 'string' ? cause : '';
  const text = raw.replace(/^[A-Z][A-Z0-9_]+:\s*/, '').trim();
  return text || fallback;
}
