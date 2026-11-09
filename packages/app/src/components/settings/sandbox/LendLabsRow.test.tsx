// @vitest-environment jsdom
// The Labs switch "Lend an App": it shows the sandbox-style Labs row, reads its own key, and saves only that key.
import { afterEach, describe, expect, it } from 'vitest';
import { fakeSandboxTransport } from '../../../lib/potato-sandbox';
import { byLabel, click, hasText, mountRows, settle, type Mounted } from '../test-utils';
import { makeSandboxRows } from './SandboxRows';

let mounted: Mounted | undefined;
afterEach(async () => {
  await mounted?.unmount();
  mounted = undefined;
});

describe('the Lend an App Labs switch', () => {
  it('sits right after the Sandbox switch and shows its title and copy', async () => {
    mounted = await mountRows(makeSandboxRows(fakeSandboxTransport()), { settings: { potato_sandbox_enabled: true } });
    await settle(6);
    const ids = [...mounted.host.querySelectorAll('[data-row]')].map((el) => el.getAttribute('data-row'));
    expect(ids.slice(0, 2)).toEqual(['potato-sandbox-labs', 'potato-lend-labs']);
    expect(hasText(mounted.host, 'Lend an App')).toBe(true);
    expect(hasText(mounted.host, 'Lend an open Windows app, like LINE, to your bot. It shows up on the bot\'s computer and the bot can use it while you watch. Windows only.')).toBe(true);
  });

  it('is off by default and saves only potato_lend_app_enabled when turned on', async () => {
    mounted = await mountRows(makeSandboxRows(fakeSandboxTransport()), { settings: { potato_sandbox_enabled: true } });
    await settle(6);
    const toggle = byLabel(mounted.host, 'Lend an App');
    expect(toggle).toBeTruthy();
    await click(toggle!);
    await settle(4);
    expect(mounted.env.patches).toEqual([{ potato_lend_app_enabled: true }]);
  });
});
