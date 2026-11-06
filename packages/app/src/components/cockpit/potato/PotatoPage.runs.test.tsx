// @vitest-environment jsdom
// The real page, with a routine that has run: the bot panel's Routines tab lists the run history, and the link on a run
// takes the person to the bot's reply message in the thread (the message is scrolled into view and marked for a moment).
import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeCompanionTransport, type CompanionView } from '../../../lib/companion';
import { fakePotatoTransport, type PotatoState } from '../../../lib/potato';
import { fakeBotSettingsTransport } from '../../../lib/potato-botsettings';
import { fakeRoutinesTransport, ROUTINE_COPY, type RoutineRunView, type RoutineView } from '../../../lib/potato-routines';
import { fakeScreenTransport } from '../../../lib/potatoScreen';
import { usePotatoState } from '../../../hooks/usePotato';
import { PotatoPage } from './PotatoPage';
import './BotPanel';
import { bot, msg, you } from './test-helpers';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

let host: HTMLDivElement;
let root: Root;
const scrolled: Element[] = [];
beforeEach(() => {
  window.localStorage.clear();
  scrolled.length = 0;
  Element.prototype.scrollIntoView = vi.fn(function (this: Element) {
    scrolled.push(this);
  });
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const companion = { id: 'fish', name: 'Fish', avatar: 'bit', persona: 'cheerful', agentType: 'claude', agentLabel: 'Claude', folderName: 'fish', state: 'idle', stateLine: 'Here.', paneId: 'companion-fish', needs: null, pendingMemory: 0, nextCheckin: null, restarts: 0, autoStart: true } as CompanionView;

const routine: RoutineView = {
  id: 'r-1',
  bot: 'fish',
  title: 'Morning sweep',
  prompt: 'Check my inbox.',
  schedule: { kind: 'weekdays', time: '06:00' },
  summary: 'Weekdays 6am',
  status: 'active',
  paused: false,
  done: false,
  lastRunMs: null,
  nextRunMs: null,
};

function Harness({ routines, messages }: { routines: ReturnType<typeof fakeRoutinesTransport>; messages: ReturnType<typeof fakePotatoTransport> }) {
  const potato = usePotatoState(messages);
  const [openId, setOpenId] = useState<string | null>('fish');
  if (!potato.state) return null;
  return (
    <PotatoPage
      state={potato.state as PotatoState}
      version={potato.version}
      transport={messages}
      companionTransport={fakeCompanionTransport({ companions: [companion] })}
      companions={[companion]}
      openBotId={openId}
      onOpenBot={setOpenId}
      onChanged={potato.refresh}
      onCompanionsChanged={() => {}}
      onNotify={() => {}}
      onOpenSettings={() => {}}
      screenTransport={fakeScreenTransport()}
      botSettings={fakeBotSettingsTransport()}
      routines={routines}
    />
  );
}

const click = async (el: Element | null | undefined) => {
  expect(el, 'element to click').toBeTruthy();
  await act(async () => (el as HTMLElement).click());
  await act(async () => {});
};
const tab = (name: string) => [...host.querySelectorAll('[role="tab"]')].find((t) => t.textContent === name);

describe('Routine run history on the real page', () => {
  it('the link on a run opens the bot reply in the thread', async () => {
    const first = you('Routine: Morning sweep\n\nCheck my inbox.');
    const reply = msg({ id: 'm-the-reply', text: 'Nothing new in your inbox.' });
    const later = msg({ id: 'm-later', text: 'Anything else?' });
    const transport = fakePotatoTransport({ bots: [bot()], messages: { fish: [first, reply, later] } });
    const at = new Date(2026, 10, 2, 6, 0, 5).getTime();
    const runs: RoutineRunView[] = [{ routineId: 'r-1', atMs: at, result: 'ok', text: 'The bot replied.', replyId: 'm-the-reply' }];
    const routines = fakeRoutinesTransport([routine], undefined, { runs });

    await act(async () => root.render(<Harness routines={routines} messages={transport} />));
    await act(async () => {});
    expect(host.querySelector('[data-message-id="m-the-reply"]')).not.toBeNull();

    await click(host.querySelector('.ck-potato-panel-toggle'));
    await click(tab('Routines'));
    const panel = host.querySelector('[role="tabpanel"][aria-label="Routines"]')!;
    expect(panel.textContent).toContain('Morning sweep');
    await click(panel.querySelector('button[aria-expanded]'));
    expect(panel.querySelector('.ck-potato-routine-run')?.textContent).toContain('The bot replied.');
    expect(scrolled).toHaveLength(0);

    await click([...panel.querySelectorAll('button')].find((b) => b.textContent === ROUTINE_COPY.showReply));
    const target = host.querySelector('[data-message-id="m-the-reply"]')!;
    expect(scrolled).toContain(target);
    expect(target.classList.contains('is-jumped')).toBe(true);
    expect(host.querySelector('[data-message-id="m-later"]')?.classList.contains('is-jumped')).toBe(false);
  });
});
