// @vitest-environment jsdom
// Settings > Potato Bot > MCP servers, and the screen path that opens it from the Potato Bot rows.
import { afterEach, describe, expect, it } from 'vitest';
import { fakePotatoTransport } from '../../../lib/potato';
import { fakeMcpTransport } from '../../../lib/potato-mcp';
import { fakeUsageTransport } from '../../../lib/potato-usage';
import { makePotatoRows } from '../sections/PotatoSection';
import { buttonByText, byLabel, click, hasText, mountRows, settle, typeInto, type Mounted } from '../test-utils';
import { McpServersSection } from './McpServersSection';

let m: Mounted;
afterEach(async () => {
  await m?.unmount();
});

async function mountSection(mcp = fakeMcpTransport()) {
  m = await mountRows([{ id: 'x', label: 'x', hint: '', keywords: '', group: 'g', render: () => <McpServersSection botId="b1" transport={mcp} /> }]);
  await settle(4);
  return mcp;
}

describe('MCP servers section', () => {
  it('shows the intro, Computer ready, and the catalog', async () => {
    await mountSection();
    for (const t of ['MCP servers', 'These tools let the bot use programs on its computer.', 'Computer', 'Ready', 'Files', 'Memory', 'Browser']) expect(hasText(m.host, t)).toBe(true);
  });

  it('shows the reason when the computer is not ready', async () => {
    await mountSection(fakeMcpTransport({ ready: false, reason: 'The sandbox is stopped.' }));
    expect(hasText(m.host, 'The sandbox is stopped.')).toBe(true);
  });

  it('adds from the catalog and then shows Added', async () => {
    await mountSection();
    await click(byLabel(m.host, 'Add Memory'));
    await settle(3);
    expect(byLabel(m.host, 'Add Memory')).toBeNull();
    expect(hasText(m.host, 'Added')).toBe(true);
    expect(byLabel(m.host, 'Remove memory')).not.toBeNull();
  });

  it('rejects a bad name, then adds a custom server and clears the env text', async () => {
    const mcp = await mountSection();
    const name = byLabel<HTMLInputElement>(m.host, 'Server name')!;
    await typeInto(name, 'Bad Name');
    await typeInto(byLabel<HTMLInputElement>(m.host, 'Server command')!, 'node');
    await click(buttonByText(m.host, 'Add server'));
    expect(hasText(m.host, 'lowercase letters')).toBe(true);
    expect(hasText(m.host, "Custom servers run inside the bot's computer and can reach what that computer's walls allow.")).toBe(true);
    await typeInto(name, 'mine');
    await typeInto(byLabel<HTMLTextAreaElement>(m.host, 'Server arguments')!, 'a.js\n--x');
    await typeInto(byLabel<HTMLTextAreaElement>(m.host, 'Server keys')!, 'TOKEN=secret');
    await click(buttonByText(m.host, 'Add server'));
    await settle(3);
    expect(mcp.received.mine).toEqual({ TOKEN: 'secret' });
    expect(byLabel<HTMLTextAreaElement>(m.host, 'Server keys')!.value).toBe('');
    expect(hasText(m.host, 'Keys: TOKEN')).toBe(true);
    expect(hasText(m.host, 'secret')).toBe(false);
  });

  it('tests, toggles and removes an installed server; shows errors as one line', async () => {
    const mcp = await mountSection(fakeMcpTransport({ servers: [{ name: 'mine', command: 'node', args: [], envKeys: [], enabled: true, connector: false, alwaysAllowReads: false, url: null, local: false, allowTools: [], blockTools: [], signedIn: false, status: 'ready', toolCount: 2, note: null }] }));
    expect(hasText(m.host, 'Ready · 2 tools')).toBe(true);
    await click(byLabel(m.host, 'Test mine'));
    await settle(3);
    expect(hasText(m.host, 'Tools: read_file, write_file')).toBe(true);
    await click(byLabel(m.host, 'Enable mine'));
    await settle(3);
    expect(hasText(m.host, 'Off')).toBe(true);
    mcp.failNext('MCP_BOOM: it broke');
    await click(byLabel(m.host, 'Remove mine'));
    await settle(3);
    expect(hasText(m.host, 'it broke')).toBe(true);
    await click(byLabel(m.host, 'Remove mine'));
    await settle(3);
    expect(byLabel(m.host, 'Remove mine')).toBeNull();
  });
});

describe('screen path: Potato Bot rows', () => {
  it('has an MCP servers row that opens the section with Computer and the catalog', async () => {
    const potato = fakePotatoTransport({ bots: [{ id: 'bot-1', name: 'Fish', talkPersona: 'fish', brainState: 'stopped', lastError: null } as never] });
    const rows = makePotatoRows(potato, undefined, fakeUsageTransport({} as never), fakeMcpTransport());
    m = await mountRows(rows, { settings: { potato_bot_enabled: true } as never });
    await settle(6);
    expect(hasText(m.host, 'MCP servers')).toBe(true);
    await click(byLabel(m.host, 'Open MCP servers'));
    await settle(6);
    expect(hasText(document.body, 'Computer')).toBe(true);
    for (const t of ['Files', 'Memory', 'Browser']) expect(hasText(document.body, t)).toBe(true);
  });
});
