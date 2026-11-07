// @vitest-environment jsdom
// Settings > Phone: devices, revoke, the pairing QR, the tunnel switch and its plain sentence, the new-QR notice,
// the firewall button, the port and address messages, the "Can't connect?" flow, and that no token text shows.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PairingStart, PhoneStatus } from '../../lib/phone-api';
import { buttonByText, byLabel, click, settle, typeInto } from './test-utils';

const { invoke, listenMock } = vi.hoisted(() => ({ invoke: vi.fn(), listenMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }));

import { PhoneSettings, TUNNEL_SENTENCE } from './PhoneSettings';

const NOW_SECS = Math.floor(Date.now() / 1000);

function baseStatus(over: Partial<PhoneStatus> = {}): PhoneStatus {
  return {
    enabled: true,
    port: 1420,
    portError: null,
    pcName: 'DESK',
    lanUrls: ['http://192.168.1.5:1420'],
    customAddress: null,
    customWarning: null,
    tunnel: { on: false, phase: 'off', url: null, error: null },
    firewall: { state: 'ok', message: '' },
    devices: [
      { id: 'd1', name: 'Mia phone', pairedAtSecs: NOW_SECS - 86400 * 3, lastSeenSecs: NOW_SECS - 300, expired: false, caps: [] },
      { id: 'd2', name: 'Old tablet', pairedAtSecs: NOW_SECS - 86400 * 30, lastSeenSecs: null, expired: false, caps: [] },
    ],
    ...over,
  };
}

const PAIR: PairingStart = {
  code: 'ABCD1234EFGH',
  expiresAtSecs: NOW_SECS + 600,
  qrUrl: 'http://192.168.1.5:1420/pair?c=ONETIMECODE',
  candidates: ['http://192.168.1.5:1420'],
  via: 'lan',
};

let status: PhoneStatus;
let pairVia = 'lan';
let calls: Array<[string, unknown]>;
let failures: Record<string, string>;
let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  status = baseStatus();
  pairVia = 'lan';
  calls = [];
  failures = {};
  listenMock.mockReset();
  listenMock.mockResolvedValue(() => undefined);
  invoke.mockReset();
  invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    calls.push([command, args]);
    if (failures[command]) throw failures[command];
    if (command === 'phone_status') return status;
    if (command === 'phone_pairing_start') return { ...PAIR, via: (args?.via as string) ?? pairVia };
    if (command === 'phone_tunnel_set') {
      status = { ...status, tunnel: args?.on ? { on: true, phase: 'running', url: 'https://one.trycloudflare.com', error: null } : { on: false, phase: 'off', url: null, error: null } };
      return status;
    }
    return status;
  });
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

const mount = async () => {
  await act(async () => root.render(<PhoneSettings />));
  await settle(3);
};
const commands = (name: string) => calls.filter(([c]) => c === name);
const text = () => host.textContent ?? '';
const modal = () => document.querySelector('[role="dialog"]');
const sheetButton = (label: string) => [...document.querySelectorAll('.st-sheet button')].find((b) => b.textContent?.trim() === label);

describe('Settings > Phone', () => {
  it('lists the paired phones with when they were paired and last used', async () => {
    await mount();
    expect(text()).toContain('Mia phone');
    expect(text()).toContain('Old tablet');
    expect(text()).toContain('Last used 5 minutes ago');
    expect(text()).toContain('Not used yet');
    expect(text()).toContain('Paired');
    expect(text()).toContain('http://192.168.1.5:1420');
  });

  it('asks before it revokes, and only then calls the command', async () => {
    await mount();
    await click(byLabel(host, 'Revoke Mia phone'));
    expect(modal()?.textContent).toContain('This phone will be signed out at once');
    expect(commands('phone_device_revoke')).toHaveLength(0);
    await click(sheetButton('Cancel'));
    expect(commands('phone_device_revoke')).toHaveLength(0);
    await click(byLabel(host, 'Revoke Mia phone'));
    await click(sheetButton('Revoke'));
    await settle(2);
    expect(commands('phone_device_revoke')[0][1]).toEqual({ deviceId: 'd1' });
  });

  it('shows the QR and the code in groups of four, and starts pairing over the Wi-Fi', async () => {
    await mount();
    await click(buttonByText(host, 'Show pairing QR'));
    await settle(2);
    const img = host.querySelector<HTMLImageElement>('img[alt="QR code to pair your phone"]');
    expect(img?.src.startsWith('data:image/svg+xml')).toBe(true);
    expect(text()).toContain('ABCD 1234 EFGH');
    expect(text()).toContain('Works once, for 10 minutes');
    expect(text()).toContain('Scan this with your phone camera.');
    expect(commands('phone_pairing_start')[0][1]).toEqual({ via: 'lan', relinkDeviceId: undefined });
    expect(buttonByText(host, 'Make a new code')).toBeTruthy();
  });

  it('pairs through the tunnel when it runs and there is no Wi-Fi address, and shows the way picker only with several ways', async () => {
    status = baseStatus({ lanUrls: [], tunnel: { on: true, phase: 'running', url: 'https://one.trycloudflare.com', error: null } });
    await mount();
    expect(byLabel(host, 'How the phone will reach this PC')).toBeNull();
    await click(buttonByText(host, 'Show pairing QR'));
    await settle(2);
    expect(commands('phone_pairing_start')[0][1]).toMatchObject({ via: 'tunnel' });
  });

  it('shows a way picker when more than one way is available', async () => {
    status = baseStatus({ tunnel: { on: true, phase: 'running', url: 'https://one.trycloudflare.com', error: null } });
    await mount();
    const select = byLabel<HTMLSelectElement>(host, 'How the phone will reach this PC');
    expect([...select!.options].map((o) => o.textContent)).toEqual(['Same Wi-Fi', 'Anywhere']);
  });

  it('Re-link starts pairing for that phone', async () => {
    await mount();
    await click(byLabel(host, 'Re-link Old tablet'));
    await settle(2);
    expect(commands('phone_pairing_start')[0][1]).toMatchObject({ relinkDeviceId: 'd2' });
  });

  it('the tunnel switch asks the plain sentence first; Cancel calls nothing; Turn on calls the command', async () => {
    await mount();
    await click(byLabel(host, 'Reach from outside your Wi-Fi'));
    expect(modal()?.textContent).toContain(TUNNEL_SENTENCE);
    expect(modal()?.textContent).toContain('Cloudflare says this free tunnel is for testing and may stop at any time.');
    await click(sheetButton('Cancel'));
    expect(commands('phone_tunnel_set')).toHaveLength(0);
    await click(byLabel(host, 'Reach from outside your Wi-Fi'));
    await click(sheetButton('Turn on'));
    await settle(2);
    expect(commands('phone_tunnel_set')[0][1]).toEqual({ on: true });
    expect(text()).toContain('https://one.trycloudflare.com');
  });

  it('turning the tunnel off needs no question', async () => {
    status = baseStatus({ tunnel: { on: true, phase: 'running', url: 'https://one.trycloudflare.com', error: null } });
    await mount();
    await click(byLabel(host, 'Reach from outside your Wi-Fi'));
    await settle(2);
    expect(modal()).toBeNull();
    expect(commands('phone_tunnel_set')[0][1]).toEqual({ on: false });
  });

  it('shows the tunnel error sentence', async () => {
    status = baseStatus({ tunnel: { on: true, phase: 'error', url: null, error: 'TUNNEL_FAILED: The tunnel program could not start.' } });
    await mount();
    expect(text()).toContain('The tunnel program could not start.');
    expect(text()).not.toContain('TUNNEL_FAILED');
  });

  it('says the address changed and makes a new QR when the tunnel address changes', async () => {
    status = baseStatus({ lanUrls: [], tunnel: { on: true, phase: 'running', url: 'https://one.trycloudflare.com', error: null } });
    listenMock.mockReset();
    let changed: () => void = () => undefined;
    listenMock.mockImplementation(async (_event: string, cb: () => void) => {
      changed = cb;
      return () => undefined;
    });
    await mount();
    await click(buttonByText(host, 'Show pairing QR'));
    await settle(3);
    expect(commands('phone_pairing_start')).toHaveLength(1);
    status = { ...status, tunnel: { on: true, phase: 'running', url: 'https://two.trycloudflare.com', error: null } };
    await act(async () => changed());
    await settle(4);
    expect(text()).toContain('The address changed. Scan the new QR');
    expect(commands('phone_pairing_start')).toHaveLength(2);
  });

  it('firewall missing: shows the message and the Allow button, which calls the command', async () => {
    status = baseStatus({ firewall: { state: 'missing', message: 'Windows Firewall may block phones on port 1420.' } });
    await mount();
    expect(text()).toContain('Windows Firewall may block phones on port 1420.');
    expect(text()).toContain('Windows will ask you to confirm');
    await click(buttonByText(host, 'Allow in Windows Firewall'));
    await settle(2);
    expect(commands('phone_firewall_allow')).toHaveLength(1);
  });

  it('no Allow button when the firewall is fine', async () => {
    await mount();
    expect(buttonByText(host, 'Allow in Windows Firewall')).toBeUndefined();
  });

  it('shows the plain sentence when the port is taken', async () => {
    failures.phone_set_port = 'PHONE_PORT_IN_USE: Another program is using port 1420. Pick another port.';
    await mount();
    await typeInto(byLabel<HTMLInputElement>(host, 'Port')!, '1421');
    await click(buttonByText(host, 'Save port'));
    await settle(2);
    expect(text()).toContain('Another program is using port 1420. Pick another port.');
    expect(text()).not.toContain('PHONE_PORT_IN_USE');
    expect(commands('phone_set_port')[0][1]).toEqual({ port: 1421 });
  });

  it('refuses a port outside 1024 to 65535 without calling the command', async () => {
    await mount();
    await typeInto(byLabel<HTMLInputElement>(host, 'Port')!, '80');
    await click(buttonByText(host, 'Save port'));
    expect(text()).toContain('Pick a number from 1024 to 65535.');
    expect(commands('phone_set_port')).toHaveLength(0);
  });

  it('saves and clears my own address, and shows the warning when it is plain http', async () => {
    status = baseStatus({ customAddress: 'http://myhost:8123', customWarning: 'This address is plain http outside your home network, so the phone token is sent unencrypted.' });
    await mount();
    expect(text()).toContain('sent unencrypted');
    expect(text()).toContain('in front of port 1420');
    await typeInto(byLabel<HTMLInputElement>(host, 'My own address')!, 'https://bot.example.com');
    await click(buttonByText(host, 'Save address'));
    await settle(2);
    expect(commands('phone_set_custom_address')[0][1]).toEqual({ address: 'https://bot.example.com' });
    await click(buttonByText(host, 'Clear'));
    await settle(2);
    expect(commands('phone_set_custom_address')[1][1]).toEqual({ address: null });
  });

  it("the Can't connect button asks, turns the tunnel on, waits until it runs, then pairs through the tunnel", async () => {
    let polls = 0;
    invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      calls.push([command, args]);
      if (command === 'phone_tunnel_set') {
        status = { ...status, tunnel: { on: true, phase: 'starting', url: null, error: null } };
        return status;
      }
      if (command === 'phone_status') {
        if (status.tunnel.phase === 'starting') polls += 1;
        if (polls >= 2) status = { ...status, tunnel: { on: true, phase: 'running', url: 'https://one.trycloudflare.com', error: null } };
        return status;
      }
      if (command === 'phone_pairing_start') return { ...PAIR, via: 'tunnel' };
      return status;
    });
    await mount();
    await click(buttonByText(host, "Can't connect? Turn on and show a new QR"));
    expect(modal()?.textContent).toContain(TUNNEL_SENTENCE);
    expect(commands('phone_tunnel_set')).toHaveLength(0);
    await click(sheetButton('Turn on'));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 2400));
    });
    await settle(3);
    expect(commands('phone_tunnel_set')[0][1]).toEqual({ on: true });
    expect(commands('phone_pairing_start')[0][1]).toMatchObject({ via: 'tunnel' });
    expect(host.querySelector('img[alt="QR code to pair your phone"]')).toBeTruthy();
  }, 15000);

  it('never shows a token or the one-time link text', async () => {
    await mount();
    await click(buttonByText(host, 'Show pairing QR'));
    await settle(2);
    expect(text().toLowerCase()).not.toContain('token:');
    expect(text()).not.toContain('ONETIMECODE');
    expect(host.innerHTML).not.toContain('ONETIMECODE');
  });
});
