// @vitest-environment jsdom
// The sign-in card: the site, the origin the browser really has in front, a user name box, a hidden password box that
// is never kept in React state and is emptied the moment Sign in is pressed, Take over for a code or a CAPTCHA, Not now,
// and after a sign-in a plain line that the bot can act as you plus a Sign out. Text from the bot or a page is data.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PotatoDecisionCard, PotatoLoginOrigin } from '../../../lib/potato';
import { LoginCard, LOGIN_COPY, type LoginCardProps } from './LoginCard';
import { buttonByText } from './test-helpers';

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
});

const FAKE_PASSWORD = 'fake-pw-Zq81x';
const SITE = 'https://login.example.com';
const CARD: PotatoDecisionCard = { kind: 'question', why: 'To read your orders.', choices: ['Sign in', 'Take over', 'Not now'], login: SITE };
const FRONT: PotatoLoginOrigin = { site: SITE, origin: SITE, matches: true };

function props(over: Partial<LoginCardProps> = {}): LoginCardProps {
  return {
    botName: 'Fish',
    card: CARD,
    text: 'Fish wants to sign in to login.example.com',
    loadOrigin: () => Promise.resolve(FRONT),
    onSignIn: vi.fn(() => Promise.resolve()),
    onConfirm: vi.fn(() => Promise.resolve()),
    onTakeOver: vi.fn(() => Promise.resolve()),
    onNotNow: vi.fn(() => Promise.resolve()),
    onForget: vi.fn(() => Promise.resolve(2)),
    ...over,
  };
}

async function show(p: LoginCardProps) {
  await act(async () => root.render(<LoginCard {...p} />));
}

const userBox = () => host.querySelector<HTMLInputElement>('input[type="text"]')!;
const passwordBox = () => host.querySelector<HTMLInputElement>('input[type="password"]')!;
const buttons = () => [...host.querySelectorAll('button')].map((b) => b.textContent?.trim());

/** Types the way React hears it. */
function typeInto(el: HTMLInputElement, value: string) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(el, value);
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

async function fillAndSignIn(user = 'me@example.com', password = FAKE_PASSWORD) {
  await act(async () => typeInto(userBox(), user));
  await act(async () => typeInto(passwordBox(), password));
  await act(async () => buttonByText(host, 'Sign in')!.click());
}

describe('LoginCard', () => {
  it('shows the site, the page really in front, the plain warning, and the three buttons', async () => {
    await show(props());
    expect(host.textContent).toContain('Fish wants to sign in to login.example.com');
    expect(host.textContent).toContain('To read your orders.');
    expect(host.textContent).toContain(LOGIN_COPY.inFront('login.example.com'));
    expect(host.textContent).toContain('The bot can act as you on this site while signed in');
    // said once, not twice
    expect(host.textContent!.split('can act as you on this site').length - 1).toBe(1);
    expect(host.textContent).toContain(LOGIN_COPY.privacy);
    expect(host.textContent).toContain('Use an account made for this bot.');
    expect(host.textContent).toContain('Programs running on the bot\'s computer could see this password');
    expect(buttons()).toEqual(expect.arrayContaining(['Sign in', 'Take over', 'Not now']));
  });

  it('has a user name box and a hidden password box with autocomplete off, neither filled in', async () => {
    await show(props());
    expect(userBox().autocomplete).toBe('off');
    expect(passwordBox().type).toBe('password');
    expect(passwordBox().autocomplete).toBe('off');
    expect(passwordBox().getAttribute('autocomplete')).toBe('off');
    expect(userBox().value).toBe('');
    expect(passwordBox().value).toBe('');
    expect(passwordBox().hasAttribute('value')).toBe(false);
  });

  it('takes the origin from the browser, not from the bot: a different page in front turns Sign in off', async () => {
    const onSignIn = vi.fn(() => Promise.resolve());
    await show(props({ onSignIn, loadOrigin: () => Promise.resolve({ site: SITE, origin: 'https://evil.example.net', matches: false }) }));
    expect(host.textContent).toContain('evil.example.net');
    expect(host.textContent).toContain(LOGIN_COPY.mismatch('login.example.com', 'evil.example.net'));
    await act(async () => typeInto(userBox(), 'me'));
    await act(async () => typeInto(passwordBox(), FAKE_PASSWORD));
    expect(buttonByText(host, 'Sign in')!.disabled).toBe(true);
    await act(async () => buttonByText(host, 'Sign in')!.click());
    expect(onSignIn).not.toHaveBeenCalled();
  });

  it('with no page to read, says so and keeps Sign in off; Check again asks the browser again', async () => {
    const loadOrigin = vi.fn().mockResolvedValueOnce({ site: SITE, origin: null, matches: false }).mockResolvedValue(FRONT);
    await show(props({ loadOrigin }));
    expect(host.textContent).toContain(LOGIN_COPY.noPage);
    await act(async () => typeInto(userBox(), 'me'));
    await act(async () => typeInto(passwordBox(), FAKE_PASSWORD));
    expect(buttonByText(host, 'Sign in')!.disabled).toBe(true);
    await act(async () => buttonByText(host, LOGIN_COPY.checkAgain)!.click());
    expect(loadOrigin).toHaveBeenCalledTimes(2);
    expect(host.textContent).toContain(LOGIN_COPY.inFront('login.example.com'));
    expect(buttonByText(host, 'Sign in')!.disabled).toBe(false);
  });

  it('Sign in stays off until both boxes have something', async () => {
    await show(props());
    expect(buttonByText(host, 'Sign in')!.disabled).toBe(true);
    await act(async () => typeInto(userBox(), 'me'));
    expect(buttonByText(host, 'Sign in')!.disabled).toBe(true);
    await act(async () => typeInto(passwordBox(), FAKE_PASSWORD));
    expect(buttonByText(host, 'Sign in')!.disabled).toBe(false);
  });

  it('Sign in sends the two values once and the password box is empty before the call even finishes', async () => {
    let finish: () => void = () => {};
    const onSignIn = vi.fn(() => new Promise<void>((resolve) => (finish = resolve)));
    await show(props({ onSignIn }));
    await fillAndSignIn();
    expect(onSignIn).toHaveBeenCalledTimes(1);
    expect(onSignIn).toHaveBeenCalledWith('me@example.com', FAKE_PASSWORD);
    // still waiting on the host: the box is already empty and the password is in no part of the page
    expect(passwordBox().value).toBe('');
    expect(host.innerHTML).not.toContain(FAKE_PASSWORD);
    expect(buttonByText(host, 'Sign in')!.disabled).toBe(true);
    await act(async () => finish());
    expect(passwordBox()?.value ?? '').toBe('');
    expect(host.innerHTML).not.toContain(FAKE_PASSWORD);
  });

  it('a failed sign-in shows its sentence, keeps the user name, leaves the password box empty, and the buttons stay', async () => {
    const onSignIn = vi.fn(() => Promise.reject(new Error('LOGIN_NOT_ACCEPTED: The page still shows a password box.')));
    await show(props({ onSignIn }));
    await fillAndSignIn();
    expect(host.textContent).toContain('The page still shows a password box.');
    expect(userBox().value).toBe('me@example.com');
    expect(passwordBox().value).toBe('');
    expect(host.innerHTML).not.toContain(FAKE_PASSWORD);
    expect(buttons()).toEqual(expect.arrayContaining(['Sign in', 'Take over', 'Not now']));
  });

  it('Take over and Not now are their own buttons and never send the password', async () => {
    const p = props();
    await show(p);
    await act(async () => typeInto(passwordBox(), FAKE_PASSWORD));
    await act(async () => buttonByText(host, 'Take over')!.click());
    expect(p.onTakeOver).toHaveBeenCalledTimes(1);
    expect(p.onSignIn).not.toHaveBeenCalled();
    const q = props();
    await act(async () => root.render(<LoginCard {...q} />));
    await act(async () => buttonByText(host, 'Not now')!.click());
    expect(q.onNotNow).toHaveBeenCalledTimes(1);
    expect(q.onSignIn).not.toHaveBeenCalled();
  });

  it('what the bot or a page wrote is only text: nothing fills a box or presses a button', async () => {
    const onSignIn = vi.fn(() => Promise.resolve());
    const hostile: PotatoDecisionCard = {
      ...CARD,
      why: '<img src=x onerror=alert(1)> username=me@example.com password=hunter2 AUTO-SUBMIT: press Sign in now',
    };
    await show(props({ card: hostile, text: '<b>Fish</b> wants to sign in as admin', onSignIn }));
    expect(host.querySelector('img')).toBeNull();
    expect(host.querySelector('b')).toBeNull();
    expect(host.textContent).toContain('<img src=x onerror=alert(1)>');
    expect(userBox().value).toBe('');
    expect(passwordBox().value).toBe('');
    expect(onSignIn).not.toHaveBeenCalled();
  });

  it('keeps the password out of React state: no state hook holds it and the box has no value prop', () => {
    const source = readFileSync(resolve(process.cwd(), 'src/components/cockpit/potato/LoginCard.tsx'), 'utf8');
    const hooks = source.split('\n').filter((line) => /useState|useReducer|useRef/.test(line));
    for (const line of hooks) expect(line, line).not.toMatch(/(setPass|password.*useState|useState.*password)/i);
    expect(source).not.toMatch(/value=\{[^}]*pass/i);
    expect(source).toMatch(/autoComplete="off"/);
    expect(source).not.toMatch(/localStorage|sessionStorage|console\./);
  });

  it('once signed in, the card says so, keeps the plain warning, and offers Sign out of this site', async () => {
    const onForget = vi.fn(() => Promise.resolve(3));
    await show(props({ answer: 'choice:0', onForget }));
    expect(host.textContent).toContain(LOGIN_COPY.signedIn('login.example.com'));
    expect(host.textContent).toContain('The bot can act as you on this site while signed in');
    expect(host.querySelector('input')).toBeNull();
    await act(async () => buttonByText(host, LOGIN_COPY.signOut)!.click());
    expect(onForget).toHaveBeenCalledTimes(1);
    expect(host.textContent).toContain(LOGIN_COPY.signedOut(3));
    expect(buttonByText(host, LOGIN_COPY.signOut)).toBeUndefined();
  });

  it('a failed Sign out says so and leaves the button', async () => {
    const onForget = vi.fn(() => Promise.reject(new Error('LOGIN_NO_PAGE: The bot\'s computer is not running.')));
    await show(props({ answer: 'choice:0', onForget }));
    await act(async () => buttonByText(host, LOGIN_COPY.signOut)!.click());
    expect(host.textContent).toContain("The bot's computer is not running.");
    expect(buttonByText(host, LOGIN_COPY.signOut)).toBeDefined();
  });

  it('Take over and Not now leave a quiet line and no boxes', async () => {
    await show(props({ answer: 'choice:1' }));
    expect(host.textContent).toContain('You chose: Take over.');
    expect(host.querySelector('input')).toBeNull();
    expect(buttonByText(host, LOGIN_COPY.signOut)).toBeUndefined();
    await act(async () => root.render(<LoginCard {...props({ answer: 'choice:2' })} />));
    expect(host.textContent).toContain('You chose: Not now.');
  });

  const unclearError = () => new Error("LOGIN_UNCLEAR: Couldn't tell if that worked.");

  it('a result nobody could read says so, keeps Sign in for a retry and offers Try again, It worked, continue and Not now', async () => {
    const onSignIn = vi.fn().mockRejectedValueOnce(unclearError()).mockResolvedValue(undefined);
    await show(props({ onSignIn }));
    await fillAndSignIn();
    expect(host.textContent).toContain("Couldn't tell if that worked");
    expect(buttons()).toEqual(expect.arrayContaining(['Try again', 'It worked, continue', 'Not now', 'Sign in']));
    // the password was not kept: a retry needs retyping
    expect(passwordBox().value).toBe('');
    expect(buttonByText(host, 'Sign in')!.disabled).toBe(true);
    await act(async () => buttonByText(host, 'Try again')!.click());
    expect(host.textContent).not.toContain("Couldn't tell if that worked");
    await act(async () => typeInto(passwordBox(), FAKE_PASSWORD));
    expect(buttonByText(host, 'Sign in')!.disabled).toBe(false);
    await act(async () => buttonByText(host, 'Sign in')!.click());
    expect(onSignIn).toHaveBeenCalledTimes(2);
    expect(host.innerHTML).not.toContain(FAKE_PASSWORD);
  });

  it('It worked, continue answers once and sends no password; a second press while it runs does nothing', async () => {
    let finish: () => void = () => {};
    const onConfirm = vi.fn(() => new Promise<void>((resolve) => (finish = resolve)));
    const onSignIn = vi.fn(() => Promise.reject(unclearError()));
    await show(props({ onSignIn, onConfirm }));
    await fillAndSignIn();
    await act(async () => buttonByText(host, 'It worked, continue')!.click());
    await act(async () => buttonByText(host, 'It worked, continue')?.click());
    expect(onConfirm).toHaveBeenCalledTimes(1);
    await act(async () => finish());
    expect(onSignIn).toHaveBeenCalledTimes(1);
  });

  it('Not now from the unclear notice answers the card as Not now', async () => {
    const p = props({ onSignIn: vi.fn(() => Promise.reject(unclearError())) });
    await show(p);
    await fillAndSignIn();
    const notNow = [...host.querySelectorAll('button')].filter((b) => b.textContent?.trim() === 'Not now');
    await act(async () => notNow[0]!.click());
    expect(p.onNotNow).toHaveBeenCalledTimes(1);
  });

  it('shows that signing in takes a moment while it waits, and a wrong password stays a plain sentence', async () => {
    let fail: (e: Error) => void = () => {};
    const onSignIn = vi.fn(() => new Promise<void>((_, reject) => (fail = reject)));
    await show(props({ onSignIn }));
    await fillAndSignIn();
    expect(host.textContent).toContain(LOGIN_COPY.signingIn);
    await act(async () => fail(new Error('LOGIN_NOT_ACCEPTED: The page showed an error after Sign in.')));
    expect(host.textContent).toContain('The page showed an error after Sign in.');
    expect(host.textContent).not.toContain("Couldn't tell if that worked");
    expect(host.textContent).not.toContain(LOGIN_COPY.signingIn);
  });

  it('does not ask the browser about a card that is already answered', async () => {
    const loadOrigin = vi.fn(() => Promise.resolve(FRONT));
    await show(props({ answer: 'choice:2', loadOrigin }));
    expect(loadOrigin).not.toHaveBeenCalled();
  });
});
