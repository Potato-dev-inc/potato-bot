// @vitest-environment jsdom
// The update notice and the Settings row at screen level: quiet when there is nothing, one button, nothing installs before
// the press, a running job is warned about first, and a failed update says why in plain words.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { UpdateOffer, UpdateTransport } from '../../lib/app-update';
import { buttonByText, click, hasText, settle } from '../settings/test-utils';
import { UpdateRow } from '../settings/sections/UpdateRow';
import { AppUpdateHost } from './AppUpdateHost';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root | undefined;
let host: HTMLDivElement | undefined;
afterEach(async () => {
  if (root) await act(async () => root!.unmount());
  host?.remove();
  root = host = undefined;
});

const OFFER: UpdateOffer = { currentVersion: '1.0.2', version: '1.0.3', notes: null };

function transport(over: Partial<UpdateTransport> = {}) {
  const t = {
    check: vi.fn(async () => OFFER as UpdateOffer | null),
    busy: vi.fn(async () => [] as string[]),
    install: vi.fn(async (_confirm: boolean) => undefined),
    ...over,
  };
  return t as UpdateTransport & { check: ReturnType<typeof vi.fn>; busy: ReturnType<typeof vi.fn>; install: ReturnType<typeof vi.fn> };
}

async function mount(node: React.ReactElement) {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => root!.render(node));
  await settle(6);
  return host;
}

describe('the notice', () => {
  it('is quiet when this is the newest version', async () => {
    const h = await mount(<AppUpdateHost transport={transport({ check: vi.fn(async () => null) })} />);
    expect(h.querySelector('.app-update-notice')).toBeNull();
  });

  it('is quiet when the quiet check cannot reach the network', async () => {
    const h = await mount(<AppUpdateHost transport={transport({ check: vi.fn(async () => Promise.reject(new Error('Could not reach the update server.'))) })} />);
    expect(h.querySelector('.app-update-notice')).toBeNull();
  });

  it('says Update available with the version, and installs nothing until the button is pressed', async () => {
    const t = transport();
    const h = await mount(<AppUpdateHost transport={t} />);
    expect(hasText(h, 'Update available: 1.0.3')).toBe(true);
    expect(t.install).not.toHaveBeenCalled();
    await click(buttonByText(h, 'Update'));
    await settle();
    expect(t.install).toHaveBeenCalledWith(false);
    expect(hasText(h, 'Updating.')).toBe(true);
  });

  it('warns first when a job is running, and goes on only after the second press', async () => {
    const t = transport({ busy: vi.fn(async () => ['fish']) });
    const h = await mount(<AppUpdateHost transport={t} />);
    await click(buttonByText(h, 'Update'));
    await settle();
    expect(t.install).not.toHaveBeenCalled();
    expect(hasText(h, 'working on a job')).toBe(true);
    await click(buttonByText(h, 'Update anyway'));
    await settle();
    expect(t.install).toHaveBeenCalledWith(true);
  });

  it('Not now goes back to the offer without installing', async () => {
    const t = transport({ busy: vi.fn(async () => ['fish']) });
    const h = await mount(<AppUpdateHost transport={t} />);
    await click(buttonByText(h, 'Update'));
    await settle();
    await click(buttonByText(h, 'Not now'));
    await settle();
    expect(hasText(h, 'Update available: 1.0.3')).toBe(true);
    expect(t.install).not.toHaveBeenCalled();
  });

  it('a bad signature is refused with a plain sentence and the app carries on', async () => {
    const t = transport({
      install: vi.fn(async () => Promise.reject(new Error('The update did not pass its safety check, so it was not installed. Your current version keeps working.'))),
    });
    const h = await mount(<AppUpdateHost transport={t} />);
    await click(buttonByText(h, 'Update'));
    await settle();
    expect(hasText(h, 'did not pass its safety check')).toBe(true);
    expect(buttonByText(h, 'Try again')).toBeDefined();
  });

  it('a network failure during the install says so in plain words', async () => {
    const t = transport({ install: vi.fn(async () => Promise.reject(new Error('NETWORK: Could not reach the update server. Check your internet and try again. Your current version keeps working.'))) });
    const h = await mount(<AppUpdateHost transport={t} />);
    await click(buttonByText(h, 'Update'));
    await settle();
    expect(hasText(h, 'Could not reach the update server')).toBe(true);
    expect(hasText(h, 'NETWORK')).toBe(false);
  });
});

describe('the Settings row', () => {
  it('does not check until Check for updates is pressed, and says when there is nothing new', async () => {
    const t = transport({ check: vi.fn(async () => null) });
    const h = await mount(<UpdateRow transport={t} />);
    expect(t.check).not.toHaveBeenCalled();
    await click(buttonByText(h, 'Check for updates'));
    await settle();
    expect(t.check).toHaveBeenCalledTimes(1);
    expect(hasText(h, 'You have the newest version.')).toBe(true);
  });

  it('shows the offer and the plain error of a failed check', async () => {
    const h = await mount(<UpdateRow transport={transport()} />);
    await click(buttonByText(h, 'Check for updates'));
    await settle();
    expect(hasText(h, 'Update available: 1.0.3')).toBe(true);
    await act(async () => root!.unmount());
    root = undefined;
    host?.remove();
    const h2 = await mount(<UpdateRow transport={transport({ check: vi.fn(async () => Promise.reject(new Error('No update information was found right now. Try again later.'))) })} />);
    await click(buttonByText(h2, 'Check for updates'));
    await settle();
    expect(hasText(h2, 'No update information was found')).toBe(true);
  });
});
