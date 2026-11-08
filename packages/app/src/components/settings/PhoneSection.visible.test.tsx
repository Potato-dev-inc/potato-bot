// @vitest-environment jsdom
// Settings > Phone must not be blank. settings.css hides a group whose rows have no `.st-rowwrap > .st-row` (or
// `> section`) child, and a row with no such child; the panel used to be one wrapper <div>, and its loading and
// "could not read" lines were bare paragraphs, so the whole page drew nothing (jsdom does not run CSS, so the rule is
// checked here by its own selectors).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mountRows, settle, type Mounted } from './test-utils';

const { invoke, listenMock } = vi.hoisted(() => ({ invoke: vi.fn(), listenMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }));

import { phoneRows } from './sections/PhoneSection';

const STATUS = {
  enabled: true,
  port: 1421,
  portError: null,
  pcName: 'DESK',
  lanUrls: ['http://192.168.1.5:1421'],
  customAddress: null,
  customWarning: null,
  tunnel: { on: false, phase: 'off', url: null, error: null },
  firewall: { state: 'ok', message: '' },
  devices: [],
};

// The two rules from src/styles/settings.css that hide a row and a group (jsdom cannot run :has(> ...) itself).
const rowDrawn = (wrap: Element) => [...wrap.children].some((c) => c.matches('.st-row, section'));
const groupDrawn = (group: Element) => [...group.querySelectorAll('.st-rowwrap')].some(rowDrawn);

let m: Mounted | null = null;
beforeEach(() => {
  listenMock.mockReset();
  listenMock.mockResolvedValue(() => undefined);
});
afterEach(async () => {
  await m?.unmount();
  m = null;
});

function expectDrawn(host: HTMLElement) {
  const wrap = host.querySelector('.st-rowwrap')!;
  const group = host.querySelector('.st-group')!;
  expect(rowDrawn(wrap), 'row would be display:none').toBe(true);
  expect(groupDrawn(group), 'group would be display:none').toBe(true);
}

describe('Settings > Phone is never blank', () => {
  it('draws when the status loads', async () => {
    invoke.mockImplementation(async () => STATUS);
    m = await mountRows(phoneRows);
    await settle(3);
    expect(m.host.textContent).toContain('Pair a phone');
    expectDrawn(m.host);
  });

  it('draws the loading line', async () => {
    invoke.mockImplementation(() => new Promise(() => undefined));
    m = await mountRows(phoneRows);
    expect(m.host.textContent).toContain('Loading');
    expectDrawn(m.host);
  });

  it('draws the error when the status cannot be read', async () => {
    invoke.mockImplementation(async () => {
      throw 'PHONE_FAILED: The phone server is not running.';
    });
    m = await mountRows(phoneRows);
    await settle(3);
    expect(m.host.textContent).toContain('The phone server is not running.');
    expectDrawn(m.host);
  });
});
