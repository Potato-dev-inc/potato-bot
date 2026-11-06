// MCP servers for a Potato Bot: the programs the bot may use on its computer. The names mirror the Rust commands
// `potato_mcp_*` in camelCase. Env VALUES never come back from the backend: a server view has only the key names.
// `fakeMcpTransport` is the in-memory twin for tests.
import { invoke } from '@tauri-apps/api/core';

export interface CatalogEntry {
  id: string;
  name: string;
  title: string;
  description: string;
  command: string;
  args: string[];
  envKeys: string[];
  /** A service that needs a token (listed under Connectors) rather than a plain tool server. */
  connector: boolean;
  /** What the person types for a connector: the variable name, how to call it, a hint. */
  fields: { key: string; label: string; help: string }[];
  maker: string | null;
  docs: string | null;
  installed: boolean;
}

export type McpStatus = 'off' | 'ready' | 'starting' | 'failed' | 'unknown';

export interface McpServerView {
  name: string;
  command: string;
  args: string[];
  envKeys: string[];
  enabled: boolean;
  connector: boolean;
  /** Connector only: reads run without a card. */
  alwaysAllowReads: boolean;
  /** A tool server by link: the link (never a token). `null` for a server in the computer. */
  url: string | null;
  /** By link only: Local was ticked (a server on this PC or the own network). */
  local: boolean;
  /** The tools the bot may call (empty: all) and the ones it may never call. Checked by the app on every call. */
  allowTools: string[];
  blockTools: string[];
  /** By link only: signed in with the server's own sign-in. The tokens stay on the PC and never come back. */
  signedIn: boolean;
  status: McpStatus;
  toolCount: number | null;
  note: string | null;
}

export interface McpOverview {
  builtin: { name: 'computer'; ready: boolean; reason: string | null };
  catalog: CatalogEntry[];
  servers: McpServerView[];
}

export interface McpTestResult {
  ok: boolean;
  tools: string[];
  error: string | null;
}

export interface McpNewServer {
  name: string;
  command: string;
  args: string[];
  env: Record<string, string>;
}

export interface McpNewLink {
  name: string;
  url: string;
  /** Allows an address on this PC or the own network (and plain http there). */
  local: boolean;
  /** Optional. Typed once; kept on the PC and used only when the app calls this server. Never comes back. */
  token: string | null;
}

export const MCP_NAME_PATTERN = /^[a-z0-9_-]{1,32}$/;

export interface McpTransport {
  overview(botId: string): Promise<McpOverview>;
  catalogAdd(botId: string, id: string): Promise<McpOverview>;
  add(botId: string, server: McpNewServer): Promise<McpOverview>;
  /** Adds a tool server by link. The app calls it from this PC; the bot never holds the token. */
  addLink(botId: string, link: McpNewLink): Promise<McpOverview>;
  remove(botId: string, name: string): Promise<McpOverview>;
  /** Which tools of a server the bot may call: `allow` (empty means all) and `block`. */
  setTools(botId: string, name: string, allow: string[], block: string[]): Promise<McpOverview>;
  /** Opens the server's sign-in in the person's browser and waits until it is finished. `clientId` only for a server that cannot register apps by itself. */
  signIn(botId: string, name: string, clientId: string | null): Promise<McpOverview>;
  signOut(botId: string, name: string): Promise<McpOverview>;
  setEnabled(botId: string, name: string, enabled: boolean): Promise<McpOverview>;
  test(botId: string, name: string): Promise<McpTestResult>;
  /** Installs a connector on the running computer and keeps the typed values on the PC. They never come back. */
  connect(botId: string, id: string, values: Record<string, string>): Promise<McpOverview>;
  setAlwaysAllowReads(botId: string, name: string, on: boolean): Promise<McpOverview>;
}

export const tauriMcpTransport: McpTransport = {
  overview: (botId) => invoke<McpOverview>('potato_mcp_overview', { botId }),
  catalogAdd: (botId, id) => invoke<McpOverview>('potato_mcp_catalog_add', { botId, id }),
  add: (botId, server) => invoke<McpOverview>('potato_mcp_add', { botId, server }),
  addLink: (botId, link) => invoke<McpOverview>('potato_mcp_remote_add', { botId, name: link.name, url: link.url, local: link.local, token: link.token }),
  remove: (botId, name) => invoke<McpOverview>('potato_mcp_remove', { botId, name }),
  setTools: (botId, name, allow, block) => invoke<McpOverview>('potato_mcp_tools_set', { botId, name, allow, block }),
  signIn: (botId, name, clientId) => invoke<McpOverview>('potato_mcp_oauth_sign_in', { botId, name, clientId }),
  signOut: (botId, name) => invoke<McpOverview>('potato_mcp_oauth_sign_out', { botId, name }),
  setEnabled: (botId, name, enabled) => invoke<McpOverview>('potato_mcp_set_enabled', { botId, name, enabled }),
  test: (botId, name) => invoke<McpTestResult>('potato_mcp_test', { botId, name }),
  connect: (botId, id, values) => invoke<McpOverview>('potato_connector_connect', { botId, id, values }),
  setAlwaysAllowReads: (botId, name, on) => invoke<McpOverview>('potato_connector_reads', { botId, name, on }),
};

const PLAIN = { connector: false, fields: [], maker: null, docs: null };
const SEED_CATALOG: Omit<CatalogEntry, 'installed'>[] = [
  { id: 'filesystem', name: 'filesystem', title: 'Files', description: 'Read and write files on the bot computer.', command: 'npx', args: ['-y', '@modelcontextprotocol/server-filesystem', '/work'], envKeys: [], ...PLAIN },
  { id: 'memory', name: 'memory', title: 'Memory', description: 'Remember facts between chats.', command: 'npx', args: ['-y', '@modelcontextprotocol/server-memory'], envKeys: [], ...PLAIN },
  { id: 'playwright', name: 'playwright', title: 'Browser', description: 'Drive a web browser.', command: 'npx', args: ['-y', '@playwright/mcp'], envKeys: [], ...PLAIN },
  {
    id: 'github',
    name: 'github',
    title: 'GitHub',
    description: "Repositories, issues, pull requests and code search, through GitHub's own MCP server.",
    command: '/home/pm/.connectors/github/github-mcp-server',
    args: ['stdio'],
    envKeys: ['GITHUB_PERSONAL_ACCESS_TOKEN'],
    connector: true,
    fields: [{ key: 'GITHUB_PERSONAL_ACCESS_TOKEN', label: 'Personal access token', help: 'Create a fine-grained token at github.com/settings/tokens.' }],
    maker: 'GitHub (official)',
    docs: 'https://github.com/github/github-mcp-server',
  },
  {
    id: 'notion',
    name: 'notion',
    title: 'Notion',
    description: "Search and edit pages and databases. Notion says this package is no longer actively maintained.",
    command: '/home/pm/.connectors/node_modules/.bin/notion-mcp-server',
    args: [],
    envKeys: ['NOTION_TOKEN'],
    connector: true,
    fields: [{ key: 'NOTION_TOKEN', label: 'Integration token', help: 'Create an internal integration at notion.so/profile/integrations.' }],
    maker: 'Notion (official, no longer actively maintained)',
    docs: 'https://github.com/makenotion/notion-mcp-server',
  },
];

export interface FakeMcpTransport extends McpTransport {
  /** Env values the fake received on `add`, by server name (the real backend keeps them out of every view). */
  received: Record<string, Record<string, string>>;
  failNext(message: string): void;
  /** Client ids the fake received on `signIn`, by server name. */
  signInClientIds: Record<string, string | null>;
}

export function fakeMcpTransport(seed: { ready?: boolean; reason?: string | null; servers?: McpServerView[] } = {}): FakeMcpTransport {
  const servers: McpServerView[] = (seed.servers ?? []).map((s) => ({ ...s }));
  const received: Record<string, Record<string, string>> = {};
  const signInClientIds: Record<string, string | null> = {};
  let failure: string | null = null;
  const view = (): McpOverview => ({
    builtin: { name: 'computer', ready: seed.ready ?? true, reason: seed.reason ?? null },
    catalog: SEED_CATALOG.map((c) => ({ ...c, installed: servers.some((s) => s.name === c.name) })),
    servers: servers.map((s) => ({ ...s })),
  });
  const guard = () => {
    if (failure) {
      const message = failure;
      failure = null;
      throw new Error(message);
    }
  };
  return {
    received,
    signInClientIds,
    failNext: (message) => {
      failure = message;
    },
    overview: async () => {
      guard();
      return view();
    },
    catalogAdd: async (_b, id) => {
      guard();
      const entry = SEED_CATALOG.find((c) => c.id === id);
      if (!entry) throw new Error('Unknown catalog entry.');
      if (!servers.some((s) => s.name === entry.name)) {
        servers.push({ name: entry.name, command: entry.command, args: [...entry.args], envKeys: [...entry.envKeys], enabled: true, connector: false, alwaysAllowReads: false, url: null, local: false, allowTools: [], blockTools: [], signedIn: false, status: 'unknown', toolCount: null, note: null });
      }
      return view();
    },
    add: async (_b, server) => {
      guard();
      if (servers.some((s) => s.name === server.name)) throw new Error('A server with that name already exists.');
      received[server.name] = { ...server.env };
      servers.push({ name: server.name, command: server.command, args: [...server.args], envKeys: Object.keys(server.env), enabled: true, connector: false, alwaysAllowReads: false, url: null, local: false, allowTools: [], blockTools: [], signedIn: false, status: 'unknown', toolCount: null, note: null });
      return view();
    },
    addLink: async (_b, link) => {
      guard();
      if (!MCP_NAME_PATTERN.test(link.name)) throw new Error('MCP_BAD_NAME: A server name is 1 to 32 letters, digits, - or _ (lower case).');
      if (servers.some((s) => s.name === link.name)) throw new Error('MCP_NAME_TAKEN: A tool server with that name already exists.');
      const lower = link.url.trim().toLowerCase();
      if (!lower.startsWith('https://') && !(link.local && lower.startsWith('http://'))) throw new Error('MCP_URL_SCHEME: A tool server link must start with https://.');
      if (!link.local && /^https:\/\/(localhost|127\.|10\.|192\.168\.|\[)/.test(lower)) throw new Error('MCP_URL_PRIVATE: That address is on a private network or this computer, so it is not opened.');
      if (link.token) received[link.name] = { TOKEN: link.token };
      servers.push({ name: link.name, command: '', args: [], envKeys: link.token ? ['TOKEN'] : [], enabled: true, connector: true, alwaysAllowReads: false, url: link.url.trim(), local: link.local, allowTools: [], blockTools: [], signedIn: false, status: 'unknown', toolCount: null, note: null });
      return view();
    },
    signIn: async (_b, name, clientId) => {
      guard();
      const s = servers.find((x) => x.name === name && x.url !== null);
      if (!s) throw new Error('MCP_NOT_FOUND: No tool server by link has that name.');
      signInClientIds[name] = clientId;
      s.signedIn = true;
      return view();
    },
    signOut: async (_b, name) => {
      guard();
      const s = servers.find((x) => x.name === name);
      if (s) s.signedIn = false;
      return view();
    },
    setTools: async (_b, name, allow, block) => {
      guard();
      const s = servers.find((x) => x.name === name);
      if (!s) throw new Error('MCP_NOT_FOUND: No tool server has that name.');
      s.allowTools = [...allow];
      s.blockTools = [...block];
      return view();
    },
    remove: async (_b, name) => {
      guard();
      const i = servers.findIndex((s) => s.name === name);
      if (i >= 0) servers.splice(i, 1);
      return view();
    },
    setEnabled: async (_b, name, enabled) => {
      guard();
      const s = servers.find((x) => x.name === name);
      if (s) {
        s.enabled = enabled;
        s.status = enabled ? 'unknown' : 'off';
      }
      return view();
    },
    connect: async (_b, id, values) => {
      guard();
      const entry = SEED_CATALOG.find((c) => c.id === id && c.connector);
      if (!entry) throw new Error('Unknown connector.');
      if (!entry.fields.every((f) => (values[f.key] ?? '').trim() !== '')) throw new Error('Fill in every field of the connector.');
      received[entry.name] = { ...values };
      servers.push({ name: entry.name, command: entry.command, args: [...entry.args], envKeys: [...entry.envKeys], enabled: true, connector: true, alwaysAllowReads: false, url: null, local: false, allowTools: [], blockTools: [], signedIn: false, status: 'unknown', toolCount: null, note: null });
      return view();
    },
    setAlwaysAllowReads: async (_b, name, on) => {
      guard();
      const s = servers.find((x) => x.name === name && x.connector);
      if (s) s.alwaysAllowReads = on;
      return view();
    },
    test: async (_b, name) => {
      guard();
      return servers.some((s) => s.name === name) ? { ok: true, tools: ['read_file', 'write_file'], error: null } : { ok: false, tools: [], error: 'No such server.' };
    },
  };
}
