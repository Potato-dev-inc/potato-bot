// @vitest-environment jsdom
// Settings > Potato Bot > Connectors: the short list, connecting with a token typed in the app's own field, the token
// leaving the page and never coming back, Always allow reads per connector, and the screen path from the Potato rows.
import { afterEach, describe, expect, it } from 'vitest';
import { fakePotatoTransport } from '../../../lib/potato';
import { fakeMcpTransport } from '../../../lib/potato-mcp';
import { fakeUsageTransport } from '../../../lib/potato-usage';
import { makePotatoRows } from '../sections/PotatoSection';
import { byLabel, click, hasText, mountRows, settle, typeInto, type Mounted } from '../test-utils';
import { ConnectorsSection, CONNECTORS_COPY } from './ConnectorsSection';

const FAKE_TOKEN = 'fake-token-9d2c41aa07';
let m: Mounted;
afterEach(async () => {
  await m?.unmount();
});

async function mountSection(mcp = fakeMcpTransport()) {
  m = await mountRows([{ id: 'x', label: 'x', hint: '', keywords: '', group: 'g', render: () => <ConnectorsSection botId="b1" transport={mcp} /> }]);
  await settle(4);
  return mcp;
}

async function connectGithub() {
  const mcp = await mountSection();
  await typeInto(byLabel<HTMLInputElement>(m.host, 'GitHub Personal access token')!, FAKE_TOKEN);
  await click(byLabel(m.host, 'Connect GitHub'));
  await settle(4);
  return mcp;
}

describe('Connectors section', () => {
  it('lists only the services, with who makes each, and says the bot never asks for a token', async () => {
    await mountSection();
    for (const t of ['Connectors', 'GitHub', 'Notion', 'Made by: GitHub (official).', CONNECTORS_COPY.intro, CONNECTORS_COPY.text]) expect(hasText(m.host, t)).toBe(true);
    expect(hasText(m.host, 'Made by: Notion (official, no longer actively maintained).')).toBe(true);
    for (const plain of ['Files', 'Memory', 'Browser']) expect(hasText(m.host, plain)).toBe(false);
  });

  it('asks for the token in a hidden field, and Connect waits until it is filled', async () => {
    await mountSection();
    const field = byLabel<HTMLInputElement>(m.host, 'GitHub Personal access token')!;
    expect(field.type).toBe('password');
    expect(byLabel<HTMLButtonElement>(m.host, 'Connect GitHub')!.disabled).toBe(true);
    expect(hasText(m.host, CONNECTORS_COPY.privacy)).toBe(true);
    await typeInto(field, FAKE_TOKEN);
    expect(byLabel<HTMLButtonElement>(m.host, 'Connect GitHub')!.disabled).toBe(false);
  });

  it('connects: the token goes to the backend once and is nowhere on the page afterwards', async () => {
    const mcp = await connectGithub();
    expect(mcp.received.github).toEqual({ GITHUB_PERSONAL_ACCESS_TOKEN: FAKE_TOKEN });
    expect(byLabel(m.host, 'Disconnect github')).not.toBeNull();
    expect(byLabel(m.host, 'GitHub Personal access token')).toBeNull();
    expect(document.body.innerHTML.includes(FAKE_TOKEN)).toBe(false);
    expect(document.body.textContent?.includes(FAKE_TOKEN)).toBe(false);
  });

  it('a connect that fails shows one plain line and not the token', async () => {
    const mcp = await mountSection();
    mcp.failNext('CONNECTOR_INSTALL_FAILED: It could not be installed on the computer.');
    await typeInto(byLabel<HTMLInputElement>(m.host, 'GitHub Personal access token')!, FAKE_TOKEN);
    await click(byLabel(m.host, 'Connect GitHub'));
    await settle(4);
    expect(hasText(m.host, 'It could not be installed on the computer.')).toBe(true);
    expect(document.body.textContent?.includes(FAKE_TOKEN)).toBe(false);
  });

  it('does not offer Connect while the computer is stopped, and says why', async () => {
    await mountSection(fakeMcpTransport({ ready: false, reason: 'The sandbox is stopped.' }));
    expect(hasText(m.host, 'The sandbox is stopped.')).toBe(true);
    expect(byLabel<HTMLButtonElement>(m.host, 'Connect GitHub')!.disabled).toBe(true);
  });

  it('Always allow reads is per connector, off by default, and says what it does', async () => {
    const mcp = await connectGithub();
    expect(hasText(m.host, CONNECTORS_COPY.readsOff)).toBe(true);
    await click(byLabel(m.host, 'Always allow reads for github'));
    await settle(3);
    expect(hasText(m.host, CONNECTORS_COPY.readsHint)).toBe(true);
    expect((await mcp.overview('b1')).servers[0]?.alwaysAllowReads).toBe(true);
    // Notion is not connected, so it has no such switch.
    expect(byLabel(m.host, 'Always allow reads for notion')).toBeNull();
  });

  it('switches a connector off for the bot and disconnects it', async () => {
    const mcp = await connectGithub();
    await click(byLabel(m.host, 'Use github for this bot'));
    await settle(3);
    expect((await mcp.overview('b1')).servers[0]?.enabled).toBe(false);
    await click(byLabel(m.host, 'Disconnect github'));
    await settle(3);
    expect(byLabel(m.host, 'Connect GitHub')).not.toBeNull();
  });
});

describe('Add by link', () => {
  const LINK_TOKEN = 'fake-token-link-3b7e2d';

  async function fill(name: string, url: string, token = '') {
    await typeInto(byLabel<HTMLInputElement>(m.host, 'Link name')!, name);
    await typeInto(byLabel<HTMLInputElement>(m.host, 'Tool server link')!, url);
    if (token) await typeInto(byLabel<HTMLInputElement>(m.host, 'Link token')!, token);
  }

  it('has a name, a link, a hidden optional token and a Local tick that starts off, and says where the token stays', async () => {
    await mountSection();
    for (const t of [CONNECTORS_COPY.linkHeading, CONNECTORS_COPY.linkHelp, CONNECTORS_COPY.linkLocalHint, CONNECTORS_COPY.linkPrivacy]) expect(hasText(m.host, t)).toBe(true);
    expect(byLabel<HTMLInputElement>(m.host, 'Link token')!.type).toBe('password');
    expect(byLabel<HTMLElement>(m.host, CONNECTORS_COPY.linkLocal)!.getAttribute('aria-checked')).toBe('false');
  });

  it('adds a link: the token goes to the backend once and is nowhere on the page afterwards; the row shows the link, not the token', async () => {
    const mcp = await mountSection();
    await fill('tracker', 'https://mcp.example.com/mcp', LINK_TOKEN);
    await click(byLabel(m.host, 'Add link'));
    await settle(4);
    expect(mcp.received.tracker).toEqual({ TOKEN: LINK_TOKEN });
    expect(hasText(m.host, 'https://mcp.example.com/mcp')).toBe(true);
    expect(byLabel(m.host, 'Disconnect tracker')).not.toBeNull();
    expect(byLabel<HTMLInputElement>(m.host, 'Link token')!.value).toBe('');
    expect(document.body.innerHTML.includes(LINK_TOKEN)).toBe(false);
    expect(document.body.textContent?.includes(LINK_TOKEN)).toBe(false);
  });

  it('refuses a plain http link and a private address without the Local tick, with one plain line and no token', async () => {
    await mountSection();
    await fill('plain', 'http://mcp.example.com/mcp', LINK_TOKEN);
    await click(byLabel(m.host, 'Add link'));
    await settle(4);
    expect(hasText(m.host, 'must start with https://')).toBe(true);
    await fill('inside', 'https://127.0.0.1/mcp');
    await click(byLabel(m.host, 'Add link'));
    await settle(4);
    expect(hasText(m.host, 'private network or this computer')).toBe(true);
    expect(byLabel(m.host, 'Disconnect plain')).toBeNull();
    expect(document.body.textContent?.includes(LINK_TOKEN)).toBe(false);
  });

  it('with Local ticked an address on this PC is accepted and the row says Local', async () => {
    const mcp = await mountSection();
    await fill('mine', 'http://127.0.0.1:8080/mcp');
    await click(byLabel(m.host, CONNECTORS_COPY.linkLocal));
    await click(byLabel(m.host, 'Add link'));
    await settle(4);
    expect((await mcp.overview('b1')).servers.find((s) => s.name === 'mine')?.local).toBe(true);
    expect(hasText(m.host, 'http://127.0.0.1:8080/mcp')).toBe(true);
  });

  it('a name with capitals is refused before anything is sent', async () => {
    const mcp = await mountSection();
    await fill('Bad Name', 'https://mcp.example.com/mcp');
    await click(byLabel(m.host, 'Add link'));
    await settle(3);
    expect(hasText(m.host, CONNECTORS_COPY.linkNameError)).toBe(true);
    expect((await mcp.overview('b1')).servers).toHaveLength(0);
  });
});

describe('Tools per connector', () => {
  it('after Test, every tool has a switch; unticking one and saving sends the allow list, and the page says some are off', async () => {
    const mcp = await connectGithub();
    await click(byLabel(m.host, 'Test github'));
    await settle(3);
    expect(byLabel(m.host, 'Allow read_file on github')).not.toBeNull();
    expect(byLabel(m.host, 'Allow write_file on github')).not.toBeNull();
    expect(hasText(m.host, CONNECTORS_COPY.toolsHelp)).toBe(true);
    await click(byLabel(m.host, 'Allow write_file on github'));
    await click(byLabel(m.host, 'Save tools for github'));
    await settle(3);
    const server = (await mcp.overview('b1')).servers[0]!;
    expect(server.allowTools).toEqual(['read_file']);
    expect(server.blockTools).toEqual([]);
    expect(hasText(m.host, CONNECTORS_COPY.toolsLimited)).toBe(false);
  });

  const seeded = (lists: { allowTools: string[]; blockTools: string[] }) =>
    fakeMcpTransport({
      servers: [{ name: 'github', command: 'x', args: [], envKeys: ['GITHUB_PERSONAL_ACCESS_TOKEN'], enabled: true, connector: true, alwaysAllowReads: false, url: null, local: false, ...lists, signedIn: false, status: 'ready', toolCount: 2, note: null }],
    });

  it('with every tool ticked the lists are empty, which means all tools', async () => {
    const mcp = await mountSection(seeded({ allowTools: ['read_file'], blockTools: [] }));
    expect(hasText(m.host, CONNECTORS_COPY.toolsLimited)).toBe(true);
    await click(byLabel(m.host, 'Test github'));
    await settle(3);
    expect(byLabel<HTMLElement>(m.host, 'Allow write_file on github')!.getAttribute('aria-checked')).toBe('false');
    await click(byLabel(m.host, 'Allow write_file on github'));
    await click(byLabel(m.host, 'Save tools for github'));
    await settle(3);
    expect((await mcp.overview('b1')).servers[0]!.allowTools).toEqual([]);
  });

  it('shows the tools that are already off, and keeps a block list it did not write', async () => {
    const mcp = await mountSection(seeded({ allowTools: [], blockTools: ['write_file'] }));
    await click(byLabel(m.host, 'Test github'));
    await settle(3);
    expect(byLabel<HTMLElement>(m.host, 'Allow write_file on github')!.getAttribute('aria-checked')).toBe('false');
    expect(byLabel<HTMLElement>(m.host, 'Allow read_file on github')!.getAttribute('aria-checked')).toBe('true');
    await click(byLabel(m.host, 'Save tools for github'));
    await settle(3);
    expect((await mcp.overview('b1')).servers[0]!.blockTools).toEqual(['write_file']);
  });
});

describe('Sign in to a link', () => {
  async function addLinked() {
    const mcp = await mountSection();
    await typeInto(byLabel<HTMLInputElement>(m.host, 'Link name')!, 'tracker');
    await typeInto(byLabel<HTMLInputElement>(m.host, 'Tool server link')!, 'https://mcp.example.com/mcp');
    await click(byLabel(m.host, 'Add link'));
    await settle(4);
    return mcp;
  }

  it('a link has Sign in (and says the sign-in stays on this PC); only links do, not GitHub', async () => {
    await addLinked();
    expect(byLabel(m.host, 'Sign in to tracker')).not.toBeNull();
    expect(hasText(m.host, CONNECTORS_COPY.signInHelp)).toBe(true);
    expect(byLabel(m.host, 'Sign in to github')).toBeNull();
  });

  it('Sign in asks the backend, then the row says Signed in and offers Sign out; no token is on the page', async () => {
    const mcp = await addLinked();
    await click(byLabel(m.host, 'Sign in to tracker'));
    await settle(4);
    expect(mcp.signInClientIds.tracker).toBeNull();
    expect(hasText(m.host, CONNECTORS_COPY.signedIn)).toBe(true);
    expect(byLabel(m.host, 'Sign in to tracker')).toBeNull();
    await click(byLabel(m.host, 'Sign out of tracker'));
    await settle(4);
    expect(byLabel(m.host, 'Sign in to tracker')).not.toBeNull();
  });

  it('a client id typed for a service that needs one goes with the sign-in, once', async () => {
    const mcp = await addLinked();
    await typeInto(byLabel<HTMLInputElement>(m.host, 'Client id for tracker')!, '  my-client-id ');
    await click(byLabel(m.host, 'Sign in to tracker'));
    await settle(4);
    expect(mcp.signInClientIds.tracker).toBe('my-client-id');
  });

  it('a sign-in that fails shows one plain line and keeps Sign in', async () => {
    const mcp = await addLinked();
    mcp.failNext('MCP_OAUTH_NEEDS_CLIENT_ID: This server does not sign apps in by itself. It needs a client id from the service.');
    await click(byLabel(m.host, 'Sign in to tracker'));
    await settle(4);
    expect(hasText(m.host, 'needs a client id from the service')).toBe(true);
    expect(byLabel(m.host, 'Sign in to tracker')).not.toBeNull();
  });
});

describe('screen path: Potato Bot rows', () => {
  it('has a Connectors row that opens the services and Add your own', async () => {
    const potato = fakePotatoTransport({ bots: [{ id: 'bot-1', name: 'Fish', talkPersona: 'fish', brainState: 'stopped', lastError: null } as never] });
    const rows = makePotatoRows(potato, undefined, fakeUsageTransport({} as never), fakeMcpTransport());
    m = await mountRows(rows, { settings: { potato_bot_enabled: true } as never });
    await settle(6);
    await click(byLabel(m.host, 'Open Connectors'));
    await settle(6);
    for (const t of ['GitHub', 'Notion', 'Add your own']) expect(hasText(document.body, t)).toBe(true);
    expect(byLabel(document.body, 'Connect GitHub')).not.toBeNull();
  });
});
