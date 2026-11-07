// @vitest-environment jsdom
// The OpenCode Go key on the real Settings > Potato Bot screen (the rows the app mounts, not the field alone): the field
// is there, a saved key reaches the keychain command under its own name and shows only as its last 4, a wrong shape and a
// rejected key each get one plain sentence, Replace and Remove work, and the key is in no page text, call or transport.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { fakePotatoTransport, type FakePotatoTransport } from '../../../lib/potato';
import type { SecretsApi } from '../../../lib/secrets';
import { blur, buttonByText, byLabel, click, hasText, mountRows, settle, type as typeInto, type Mounted } from '../test-utils';
import { makePotatoRows } from '../sections/PotatoSection';

const KEY = 'oc-fake-go-key-0123456789-abcd';

let m: Mounted;
let fake: FakePotatoTransport;
let stored: { set: boolean; last4: string | null };
let api: SecretsApi & { setCalls: [string, string][]; removeCalls: string[] };
let answer: () => string;

function makeApi(): typeof api {
  const setCalls: [string, string][] = [];
  const removeCalls: string[] = [];
  return {
    setCalls,
    removeCalls,
    storageInfo: async () => ({ secure: true, label: 'Windows Credential Manager' }),
    getMasked: async () => ({ ...stored }),
    set: async (name, value) => {
      setCalls.push([name, value]);
      stored = { set: true, last4: value.slice(-4) };
    },
    remove: async (name) => {
      removeCalls.push(name);
      stored = { set: false, last4: null };
    },
  };
}

beforeEach(() => {
  stored = { set: false, last4: null };
  answer = () => 'Works.';
  fake = fakePotatoTransport({ goKeyCheck: () => answer() });
  api = makeApi();
});
afterEach(async () => {
  await m?.unmount();
});

async function mount() {
  m = await mountRows(makePotatoRows(fake, api), { settings: { potato_bot_enabled: true } as never });
  await settle(6);
  return m;
}

async function enter(text: string) {
  const field = byLabel<HTMLInputElement>(m.host, 'OpenCode Go key')!;
  await typeInto(field, text);
  await blur(field);
  await settle(6);
}

describe('Settings > Potato Bot > OpenCode Go key', () => {
  it('shows a key field on the screen, next to the other provider keys, as a password field', async () => {
    await mount();
    const rows = [...m.host.querySelectorAll('[data-row]')].map((el) => el.getAttribute('data-row'));
    expect(rows.indexOf('potato-go-key')).toBe(rows.indexOf('potato-key') + 1);
    const field = byLabel<HTMLInputElement>(m.host, 'OpenCode Go key')!;
    expect(field.type).toBe('password');
    expect(hasText(m.host, 'Not set.')).toBe(true);
  });

  it('a typed key reaches the keychain under its own name, is checked with the plan, and then shows only its last 4', async () => {
    await mount();
    await enter(KEY);
    expect(api.setCalls).toEqual([['potato_go_key', KEY]]);
    expect(fake.calls.some((c) => c.method === 'checkGoKey')).toBe(true);
    expect(hasText(m.host, 'Works.')).toBe(true);
    expect(m.host.textContent).toContain('•••• abcd');
    expect(m.host.innerHTML).not.toContain(KEY);
    expect(byLabel(m.host, 'OpenCode Go key')).toBeNull();
    expect(buttonByText(m.host, 'Replace')).toBeTruthy();
    // no Potato command carried it
    expect(JSON.stringify(fake.calls)).not.toContain(KEY);
  });

  it('a key that cannot be right is told in one sentence and is not stored', async () => {
    await mount();
    await enter('two words in one');
    expect(api.setCalls).toEqual([]);
    expect(m.host.querySelector('[role="alert"]')?.textContent).toContain('does not look like an OpenCode Go key');
    await enter('https://opencode.ai/auth');
    expect(api.setCalls).toEqual([]);
    expect(fake.calls.some((c) => c.method === 'checkGoKey')).toBe(false);
  });

  it('a key with an unknown prefix is not blocked: only the plan can say it is wrong', async () => {
    await mount();
    await enter('zz-unknown-prefix-0123456789');
    expect(api.setCalls).toEqual([['potato_go_key', 'zz-unknown-prefix-0123456789']]);
  });

  it('a key the plan rejects gets a plain sentence, never the key, and can be replaced or removed', async () => {
    answer = () => {
      throw new Error('POTATO_KEY_CHECK: OpenCode Go did not accept that key. Check that it is a Go key and that your Go plan is active.');
    };
    await mount();
    await enter(KEY);
    const alert = m.host.querySelector('.st-key-go [role="alert"]')?.textContent ?? '';
    expect(alert).toContain('did not accept that key');
    expect(alert).not.toContain(KEY);
    expect(m.host.innerHTML).not.toContain(KEY);
    // Replace opens the field again
    await click(buttonByText(m.host, 'Replace'));
    expect(byLabel<HTMLInputElement>(m.host, 'OpenCode Go key')).toBeTruthy();
    // Remove goes through the keychain command and clears the message
    await click(buttonByText(m.host, 'Cancel'));
    await click(buttonByText(m.host, 'Remove'));
    await settle(4);
    expect(api.removeCalls).toEqual(['potato_go_key']);
    expect(m.host.querySelector('.st-key-go [role="alert"]')).toBeNull();
  });

  it('after a reload the key is still never shown in full', async () => {
    stored = { set: true, last4: 'abcd' };
    await mount();
    expect(m.host.textContent).toContain('•••• abcd');
    expect(m.host.innerHTML).not.toContain(KEY);
  });
});
