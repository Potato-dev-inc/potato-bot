// @vitest-environment jsdom
// The computer limits row: shows this PC's suggested limits, applies a slider move at once (one call after the drag settles),
// shows what the work uses, and says plainly when the memory limit is being hit.
import { act } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeSandboxTransport, type FakeSandboxSeed } from '../../../lib/potato-sandbox';
import { buttonByText, hasText, mountRows, settle, type Mounted } from '../test-utils';
import { LIMITS_SEND_MS } from './LimitsRow';
import { makeSandboxRows } from './SandboxRows';

let mounted: Mounted | undefined;
beforeEach(() => vi.useFakeTimers({ shouldAdvanceTime: true }));
afterEach(async () => {
  await mounted?.unmount();
  mounted = undefined;
  vi.useRealTimers();
});

async function open(seed: FakeSandboxSeed = {}) {
  const fake = fakeSandboxTransport(seed);
  mounted = await mountRows(makeSandboxRows(fake), { settings: { potato_sandbox_enabled: true } });
  await settle(6);
  return { fake, host: mounted.host };
}

const slider = (h: HTMLElement, label: string) => h.querySelector<HTMLInputElement>(`input[aria-label^="${label}"]`)!;

async function drag(input: HTMLInputElement, value: number) {
  const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
  await act(async () => {
    set.call(input, String(value));
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  });
}

describe('the computer limits row', () => {
  it('shows the suggested limits for this PC and says heavy jobs run slower with lower limits', async () => {
    const { host } = await open();
    expect(hasText(host, 'CPU limit')).toBe(true);
    expect(hasText(host, 'Memory limit')).toBe(true);
    expect(slider(host, 'CPU limit').value).toBe('50');
    expect(slider(host, 'Memory limit').value).toBe('4096');
    expect(hasText(host, '50% of this PC')).toBe(true);
    expect(hasText(host, '4 GB')).toBe(true);
    expect(hasText(host, 'Heavy jobs run slower with lower limits')).toBe(true);
  });

  it('sends one change after the drag settles, with both numbers, and does not ask for a restart', async () => {
    const { fake, host } = await open();
    await drag(slider(host, 'CPU limit'), 25);
    await drag(slider(host, 'CPU limit'), 30);
    expect(fake.calls.filter((c) => c.method === 'limitsSet')).toHaveLength(0);
    await act(async () => {
      vi.advanceTimersByTime(LIMITS_SEND_MS + 50);
    });
    await settle(4);
    const sent = fake.calls.filter((c) => c.method === 'limitsSet');
    expect(sent.map((c) => c.args)).toEqual([[30, 4096]]);
    expect(fake.calls.some((c) => ['stop', 'start', 'reset'].includes(c.method))).toBe(false);
  });

  it('shows what the work uses while the sandbox runs, and the plain sentence when it does not', async () => {
    const { host } = await open({ limits: { running: true, usedCpuPercent: 12.5, usedMemoryMb: 1536 } });
    expect(host.querySelector('[data-testid="limits-use"]')!.textContent).toContain('In use now: 12.5% CPU, 1.5 GB memory.');
    await mounted!.unmount();
    mounted = undefined;
    const second = await open();
    expect(second.host.querySelector('[data-testid="limits-use"]')!.textContent).toContain('not running');
  });

  it('says when the memory limit is reached, and offers the suggested limits after a change', async () => {
    const { fake, host } = await open({ limits: { running: true, usedMemoryMb: 4000, usedCpuPercent: 3, memoryPressure: true } });
    expect(hasText(host, 'memory limit is reached')).toBe(true);
    expect(buttonByText(host, 'Use the suggested limits')).toBeUndefined();
    await drag(slider(host, 'CPU limit'), 80);
    expect(buttonByText(host, 'Use the suggested limits (50%, 4 GB)')).toBeDefined();
    expect(fake.calls.filter((c) => c.method === 'limitsSet')).toHaveLength(0);
  });
});

describe('give unused memory back to Windows', () => {
  it('is off until the person presses Turn on, then says it needs a WSL start and does not offer a second press', async () => {
    const { fake, host } = await open();
    expect(hasText(host, 'Give unused memory back to Windows')).toBe(true);
    expect(fake.calls.filter((c) => c.method === 'memoryReturnEnable')).toHaveLength(0);
    await act(async () => buttonByText(host, 'Turn on')!.click());
    await settle(4);
    expect(fake.calls.filter((c) => c.method === 'memoryReturnEnable')).toHaveLength(1);
    expect(hasText(host, 'next time WSL starts')).toBe(true);
    expect(buttonByText(host, 'Turn on')).toBeUndefined();
  });

  it("leaves a value the person chose alone and says so", async () => {
    const { fake, host } = await open({ memoryReturn: { state: 'other', value: 'dropcache' } });
    expect(hasText(host, 'already sets this to "dropcache"')).toBe(true);
    expect(buttonByText(host, 'Turn on')).toBeUndefined();
    expect(fake.calls.filter((c) => c.method === 'memoryReturnEnable')).toHaveLength(0);
  });
});
