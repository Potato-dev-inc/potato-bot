// @vitest-environment jsdom
// The sandbox page on a PC where Windows' Linux feature is not usable yet: each state says one true thing, and Set up is
// enabled only when WSL can run.
import { afterEach, describe, expect, it } from 'vitest';
import { fakeSandboxTransport, type FakeSandboxSeed } from '../../../lib/potato-sandbox';
import { buttonByText, click, hasText, mountRows, settle, type Mounted } from '../test-utils';
import { makeSandboxRows } from './SandboxRows';

let mounted: Mounted | undefined;
afterEach(async () => {
  await mounted?.unmount();
  mounted = undefined;
});

async function open(seed: FakeSandboxSeed) {
  const fake = fakeSandboxTransport(seed);
  mounted = await mountRows(makeSandboxRows(fake));
  await settle(6);
  return { fake, host: mounted.host };
}

const setUp = (h: HTMLElement) => buttonByText(h, 'Set up…') as HTMLButtonElement;

describe('the sandbox page and Windows\' Linux feature', () => {
  it('no WSL: says it is needed, offers the button, Set up is off', async () => {
    const { host } = await open({ status: { linuxFeature: false } });
    expect(hasText(host, "Needs Windows' Linux feature (admin + restart).")).toBe(true);
    expect(buttonByText(host, "Turn on Windows' Linux feature…")).toBeDefined();
    expect(setUp(host).disabled).toBe(true);
  });

  it('WSL on with no distro: no warning, no button, Set up is on', async () => {
    const { host } = await open({ status: { linuxFeature: true } });
    expect(hasText(host, 'Needs Windows')).toBe(false);
    expect(hasText(host, 'Restart your PC')).toBe(false);
    expect(buttonByText(host, "Turn on Windows' Linux feature…")).toBeUndefined();
    expect(setUp(host).disabled).toBe(false);
  });

  it('restart pending: one honest message, no "is on" beside "needs", no button, Set up is off', async () => {
    const { host } = await open({
      status: { linuxFeature: false, restartPending: true, reason: 'Restart your PC to finish, then come back.' },
    });
    expect(hasText(host, 'Restart your PC to finish, then come back.')).toBe(true);
    expect(hasText(host, "Needs Windows' Linux feature")).toBe(false);
    expect(hasText(host, 'Linux feature is on')).toBe(false);
    expect(buttonByText(host, "Turn on Windows' Linux feature…")).toBeUndefined();
    expect(setUp(host).disabled).toBe(true);
  });

  it('after the button: the restart message replaces the old note and never says "is on"', async () => {
    const { fake, host } = await open({ status: { linuxFeature: false } });
    await click(buttonByText(host, "Turn on Windows' Linux feature…"));
    await settle();
    await click(buttonByText(document.body, 'Turn on'));
    await settle(6);
    expect(fake.calls.filter((c) => c.method === 'linuxEnable')).toHaveLength(1);
    expect(hasText(host, 'Restart your PC to finish, then come back.')).toBe(true);
    expect(hasText(host, 'Linux feature is on')).toBe(false);
    expect(hasText(host, "Needs Windows' Linux feature")).toBe(false);
    expect(setUp(host).disabled).toBe(true);
  });

  it('distros present: a normal page with Set up on', async () => {
    const { host } = await open({ status: { linuxFeature: true, state: 'stopped', desktops: ['ubuntu'] } });
    expect(hasText(host, "Needs Windows' Linux feature")).toBe(false);
    expect(hasText(host, 'Restart your PC')).toBe(false);
  });
});
