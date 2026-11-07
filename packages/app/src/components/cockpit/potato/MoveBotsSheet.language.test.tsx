// @vitest-environment jsdom
// "Move bots" in Traditional Chinese: the sheet, its tabs, its guide and its promise are in Chinese, and English comes back when
// the language is switched back. The privacy sentence keeps its meaning (no keys, tokens or sign-ins).
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeCompanionTransport } from '../../../lib/companion';
import { resetLocaleForTests, setLocalePref } from '../../../lib/i18n';
import { fakePotatoTransport } from '../../../lib/potato';
import { fakePortableTransport } from '../../../lib/potato-portable';
import { MoveBotsSheet } from './MoveBotsSheet';

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  resetLocaleForTests(['en-US']);
});

const render = () =>
  act(async () =>
    root.render(<MoveBotsSheet companions={fakeCompanionTransport()} potato={fakePotatoTransport({ bots: [] })} portable={fakePortableTransport()} botCount={1} onMade={() => {}} onNotify={() => {}} onClose={() => {}} />),
  );
const tabs = () => [...host.querySelectorAll<HTMLElement>('[role="tab"]')];

describe('Move bots in Traditional Chinese', () => {
  it('shows the sheet, the tabs and the guide in Chinese, and English again after switching back', async () => {
    setLocalePref('zh-TW');
    await render();
    expect(host.textContent).toContain('移動機器人');
    expect(tabs().map((t) => t.textContent)).toEqual(['機器人檔案', 'ChatGPT 或 Claude', '備份']);
    expect(host.textContent).toContain('開啟機器人檔案…');
    expect(host.textContent).not.toContain('Open a bot file');
    await act(async () => tabs()[2].click());
    expect(host.textContent).toContain('不含金鑰、權杖或登入資訊');
    await act(async () => tabs()[1].click());
    expect(host.textContent).toContain('要複製什麼、從哪裡複製');
    expect(host.textContent).toContain('自訂指示');
    await act(async () => setLocalePref('en'));
    await render();
    expect(host.textContent).toContain('Move bots');
  });
});
