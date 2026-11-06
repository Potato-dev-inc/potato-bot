// @vitest-environment jsdom
// The sign-in card in the thread (screen level): the card the bot raised shows the origin the app reads from the browser;
// Sign in goes to the transport with the typed values and nothing else sees the password; Take over takes the screen,
// opens the computer and answers the card; a card answered earlier stays answered, with Sign out; and a plain answer from
// the thread can never be "Sign in".
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakePotatoTransport, type PotatoSettings, type PotatoTransport } from '../../../lib/potato';
import { fakeScreenTransport } from '../../../lib/potatoScreen';
import { ThreadView, type ThreadViewProps } from './ThreadView';
import { bot, buttonByText, msg } from './test-helpers';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
(globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  window.localStorage.clear();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const NOW = new Date(2026, 10, 6, 15, 0).getTime();
const SETTINGS: PotatoSettings = { v: 1, brain: 'deepseek', defaultPersona: 'pro', dailyLimitCents: 50 };
const PASSWORD = 'fake-pw-Zq81x';
const SITE = 'https://login.example.com';
const CARD = msg({
  id: 'card-1',
  text: 'Fish wants to sign in to login.example.com',
  kind: 'decision_card',
  requestId: 'c1',
  card: { kind: 'question', why: 'To read your orders.', choices: ['Sign in', 'Take over', 'Not now'], login: SITE },
});
const SIGNED_IN = msg({
  id: 'res-1',
  text: 'You signed in to login.example.com.',
  from: { kind: 'note' },
  kind: 'decision_result',
  requestId: 'c1',
  card: { kind: 'question', why: '', choices: [], login: SITE, answer: 'choice:0' },
});

function setup(transport: PotatoTransport, messages = [CARD], over: Partial<ThreadViewProps> = {}): ThreadViewProps {
  return {
    bot: bot({ id: 'fish', talkPersona: 'pro' }),
    messages,
    settings: SETTINGS,
    avatar: 'mochi',
    agentLabel: 'Claude',
    transport,
    onOpenSettings: () => {},
    onNotify: () => {},
    onChanged: () => {},
    now: NOW,
    ...over,
  };
}
const flush = () => act(async () => {});

function typeInto(el: HTMLInputElement, value: string) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(el, value);
  el.dispatchEvent(new Event('input', { bubbles: true }));
}
const userBox = () => host.querySelector<HTMLInputElement>('input[type="text"]')!;
const passwordBox = () => host.querySelector<HTMLInputElement>('input[type="password"]')!;

describe('ThreadView: sign-in card', () => {
  it('asks the app which page is really in front and shows it', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], messages: { fish: [CARD] }, login: { origin: SITE } });
    await act(async () => root.render(<ThreadView {...setup(fake)} />));
    await flush();
    expect(fake.calls.some((c) => c.method === 'loginOrigin' && c.args[0] === 'fish' && c.args[1] === 'c1')).toBe(true);
    expect(host.textContent).toContain('login.example.com');
    expect(host.textContent).toContain('The bot can act as you on this site while signed in');
    expect(host.querySelector('input[type="password"]')).not.toBeNull();
  });

  it('Sign in hands the typed values to the transport, asks the thread to read again, and the password is not in the transport log', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], messages: { fish: [CARD] }, login: { origin: SITE } });
    const submitLogin = vi.fn(fake.submitLogin);
    const onChanged = vi.fn();
    await act(async () => root.render(<ThreadView {...setup({ ...fake, submitLogin }, [CARD], { onChanged })} />));
    await flush();
    await act(async () => typeInto(userBox(), 'me@example.com'));
    await act(async () => typeInto(passwordBox(), PASSWORD));
    await act(async () => buttonByText(host, 'Sign in')!.click());
    await flush();
    expect(submitLogin).toHaveBeenCalledWith('fish', 'c1', 'me@example.com', PASSWORD);
    expect(onChanged).toHaveBeenCalled();
    expect(JSON.stringify(fake.calls)).not.toContain(PASSWORD);
    expect(host.innerHTML).not.toContain(PASSWORD);
    expect(JSON.stringify({ ...window.localStorage })).not.toContain(PASSWORD);
    expect(JSON.stringify({ ...window.sessionStorage })).not.toContain(PASSWORD);
  });

  it('a card answered earlier stays answered after a reload: the quiet line, the warning and Sign out', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], login: { origin: SITE } });
    const forgetSite = vi.fn(fake.forgetSite);
    await act(async () => root.render(<ThreadView {...setup({ ...fake, forgetSite }, [CARD, SIGNED_IN])} />));
    await flush();
    expect(host.querySelector('input')).toBeNull();
    expect(host.textContent).toContain('Signed in to login.example.com');
    expect(host.textContent).toContain('The bot can act as you on this site while signed in');
    await act(async () => buttonByText(host, 'Sign out of this site')!.click());
    await flush();
    expect(forgetSite).toHaveBeenCalledWith('fish', 'c1');
    expect(host.textContent).toContain('Signed out of this site');
  });

  it('Take over takes the screen, opens the computer, then answers the card with Take over', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], messages: { fish: [CARD] }, login: { origin: SITE } });
    const answerCard = vi.fn(fake.answerCard);
    const screen = fakeScreenTransport();
    const takeOver = vi.spyOn(screen, 'takeOver');
    const onOpenComputer = vi.fn();
    const panel = { open: true, computerShown: false, onToggle: () => {}, onOpenComputer };
    await act(async () => root.render(<ThreadView {...setup({ ...fake, answerCard }, [CARD], { screen, panel })} />));
    await flush();
    await act(async () => buttonByText(host, 'Take over')!.click());
    await flush();
    expect(takeOver).toHaveBeenCalledTimes(1);
    expect(onOpenComputer).toHaveBeenCalledTimes(1);
    expect(answerCard).toHaveBeenCalledWith('fish', 'c1', '1');
  });

  it('Not now answers with its own number and sends no password', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], messages: { fish: [CARD] }, login: { origin: SITE } });
    const answerCard = vi.fn(fake.answerCard);
    const submitLogin = vi.fn(fake.submitLogin);
    await act(async () => root.render(<ThreadView {...setup({ ...fake, answerCard, submitLogin })} />));
    await flush();
    await act(async () => typeInto(passwordBox(), PASSWORD));
    await act(async () => buttonByText(host, 'Not now')!.click());
    await flush();
    expect(answerCard).toHaveBeenCalledWith('fish', 'c1', '2');
    expect(submitLogin).not.toHaveBeenCalled();
  });

  it('the fake app, like the real one, refuses a plain answer of Sign in', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], messages: { fish: [CARD] } });
    await expect(fake.answerCard('fish', 'c1', '0')).rejects.toThrow(/Sign in works only from the Sign in button/);
  });

  it('an unclear result keeps the card with Try again, It worked, continue and Not now; It worked closes it as signed in', async () => {
    const unclear = "LOGIN_UNCLEAR: Couldn't tell if that worked. Try again.";
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], messages: { fish: [CARD] }, login: { origin: SITE, signInError: unclear } });
    const confirmLogin = vi.fn(fake.confirmLogin);
    const onChanged = vi.fn();
    await act(async () => root.render(<ThreadView {...setup({ ...fake, confirmLogin }, [CARD], { onChanged })} />));
    await flush();
    await act(async () => typeInto(userBox(), 'me'));
    await act(async () => typeInto(passwordBox(), PASSWORD));
    await act(async () => buttonByText(host, 'Sign in')!.click());
    await flush();
    expect(host.textContent).toContain("Couldn't tell if that worked");
    expect(buttonByText(host, 'Try again')).toBeDefined();
    expect(buttonByText(host, 'Sign in')).toBeDefined();
    expect(host.innerHTML).not.toContain(PASSWORD);
    await act(async () => buttonByText(host, 'It worked, continue')!.click());
    await flush();
    expect(confirmLogin).toHaveBeenCalledWith('fish', 'c1');
    expect(confirmLogin).toHaveBeenCalledTimes(1);
    expect(onChanged).toHaveBeenCalled();
    expect(JSON.stringify(fake.calls)).not.toContain(PASSWORD);
    // the thread now holds the answer: a second confirm is refused by the app
    await expect(fake.confirmLogin('fish', 'c1')).rejects.toThrow(/already answered/);
  });

  it('a failed sign-in shows the sentence and the card stays open', async () => {
    const fake = fakePotatoTransport({ bots: [bot({ id: 'fish' })], messages: { fish: [CARD] }, login: { origin: SITE, signInError: 'LOGIN_WRONG_SITE: The page in front is not that site, so nothing was filled.' } });
    await act(async () => root.render(<ThreadView {...setup(fake)} />));
    await flush();
    await act(async () => typeInto(userBox(), 'me'));
    await act(async () => typeInto(passwordBox(), PASSWORD));
    await act(async () => buttonByText(host, 'Sign in')!.click());
    await flush();
    expect(host.textContent).toContain('The page in front is not that site, so nothing was filled.');
    expect(buttonByText(host, 'Sign in')).toBeDefined();
    expect(passwordBox().value).toBe('');
  });
});
