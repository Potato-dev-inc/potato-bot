// A sign-in card in the thread: the bot is at a sign-in page and asks its owner to sign in. It shows the site, the page the
// app reads from the browser (not the bot's words), a user name box, a hidden password box and three buttons: Sign in,
// Take over (for a code or a CAPTCHA) and Not now. Sign in sends the two values to the app, which types them into the page
// in the bot's browser and answers the card; the bot never sees the password.
//
// The password is never in React state, a prop, storage or the DOM after Sign in: its box is uncontrolled (the page owns the
// text), it is read once when the button is pressed, and the box is emptied before the call starts. A failed try leaves it
// empty on purpose. Text from the bot or a page is shown as text only and never fills a box or presses a button.
import { useCallback, useEffect, useRef, useState } from 'react';
import { plainError, type PotatoDecisionCard, type PotatoLoginOrigin } from '../../../lib/potato';
import '../../../styles/cockpit-potato-cards.css';
import { choiceIndex } from './DecisionCard';

export const LOGIN_COPY = {
  warning: 'The bot can act as you on this site while signed in.',
  privacy: 'Your password goes from this box to the page. The bot never sees it, and it is not saved.',
  userName: 'User name or email',
  password: 'Password',
  passwordWarning: 'Use an account made for this bot. Programs running on the bot\'s computer could see this password while it signs in.',
  signingIn: 'Signing in… this can take a few seconds.',
  unclearTitle: "Couldn't tell if that worked",
  unclearBody: "Look at the bot's screen. If you are signed in, continue. If not, type the password again and try again.",
  tryAgain: 'Try again',
  itWorked: 'It worked, continue',
  checking: 'Checking which page is in front…',
  noPage: 'No page could be read in the bot\'s browser. Open the sign-in page first, then check again.',
  checkAgain: 'Check again',
  inFront: (site: string) => `Page in front: ${site}`,
  mismatch: (asked: string, actual: string) => `The page in front is ${actual}, not ${asked}. Sign in is off.`,
  signedIn: (site: string) => `Signed in to ${site}.`,
  signOut: 'Sign out of this site',
  signedOut: (cookies: number) => `Signed out of this site. ${cookies} saved ${cookies === 1 ? 'login was' : 'logins were'} cleared.`,
  problem: 'That did not work. Try again.',
  choices: ['Sign in', 'Take over', 'Not now'],
};

/** `https://github.com` as a person reads it. */
const plainSite = (origin: string) => origin.replace(/^https:\/\//, '');

export interface LoginCardProps {
  botName: string;
  card: PotatoDecisionCard;
  /** The card's text: what the bot wants. Shown as text. */
  text: string;
  /** The answer from the thread (`decision_result`), once there is one. */
  answer?: string | null;
  /** Reads which page the browser really has in front. */
  loadOrigin: () => Promise<PotatoLoginOrigin>;
  /** Sign in. Rejects with a sentence on failure. */
  onSignIn: (username: string, password: string) => Promise<void>;
  /** It worked, continue: answers the card as signed in. */
  onConfirm: () => Promise<void>;
  /** Take over: takes the screen and answers the card. */
  onTakeOver: () => Promise<void>;
  onNotNow: () => Promise<void>;
  /** Sign out of the site. Resolves with how many saved logins were cleared. */
  onForget: () => Promise<number>;
}

export function LoginCard({ botName, card, text, answer, loadOrigin, onSignIn, onConfirm, onTakeOver, onNotNow, onForget }: LoginCardProps) {
  const site = card.login ?? '';
  const askedSite = plainSite(site);
  const [front, setFront] = useState<PotatoLoginOrigin | 'checking'>('checking');
  const [username, setUsername] = useState('');
  // Only whether the password box has something in it; never what.
  const [boxFilled, setBoxFilled] = useState(false);
  const passwordBox = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  // The page tool could not tell whether the sign-in worked: the card stays, with its own three buttons.
  const [unclear, setUnclear] = useState(false);
  const [signedOut, setSignedOut] = useState<number | null>(null);
  const open = answer == null;

  const check = useCallback(() => {
    setFront('checking');
    loadOrigin().then(
      (seen) => setFront(seen),
      () => setFront({ site, origin: null, matches: false }),
    );
  }, [loadOrigin, site]);

  useEffect(() => {
    if (open) check();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once per card; Check again is the person's button
  }, [open]);

  const run = async (work: () => Promise<void>) => {
    if (busy) return;
    setBusy(true);
    setProblem(null);
    setUnclear(false);
    try {
      await work();
    } catch (error) {
      const raw = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
      if (raw.startsWith('LOGIN_UNCLEAR')) {
        setUnclear(true);
      } else {
        setProblem(plainError(error, LOGIN_COPY.problem));
      }
    } finally {
      setBusy(false);
    }
  };

  const signIn = () =>
    run(async () => {
      const box = passwordBox.current;
      // Read once, then the box is emptied before anything is awaited.
      const typed = box?.value ?? '';
      if (box) box.value = '';
      setBoxFilled(false);
      await onSignIn(username.trim(), typed);
    });

  const matches = front !== 'checking' && front.matches;
  const ready = open && !busy && matches && username.trim() !== '' && boxFilled;

  // ---- after an answer -------------------------------------------------------------------------------------------
  if (answer != null) {
    const picked = choiceIndex(answer) ?? -1;
    const signedIn = picked === 0;
    const line = signedIn ? LOGIN_COPY.signedIn(askedSite) : answer === 'cancelled' ? 'Cancelled.' : picked >= 0 ? `You chose: ${LOGIN_COPY.choices[picked] ?? 'Answered'}.` : 'Answered.';
    return (
      <section className="ck-potato-card ck-potato-decision ck-potato-login is-closed" aria-label={`Sign-in from ${botName}`}>
        <p className="ck-potato-card-title">{text}</p>
        <p className="ck-sub ck-potato-decision-result">{line}</p>
        {signedIn && <p className="ck-sub ck-potato-card-fact">{LOGIN_COPY.warning}</p>}
        {signedIn && signedOut === null && (
          <div className="ck-potato-card-actions">
            <button
              type="button"
              className="ck-btn ck-btn--secondary"
              disabled={busy}
              onClick={() => void run(async () => setSignedOut(await onForget()))}
            >
              {LOGIN_COPY.signOut}
            </button>
          </div>
        )}
        {signedOut !== null && <p className="ck-sub ck-potato-decision-result">{LOGIN_COPY.signedOut(signedOut)}</p>}
        {problem && (
          <p className="ck-sub ck-potato-card-problem" role="status">
            {problem}
          </p>
        )}
      </section>
    );
  }

  // ---- open ---------------------------------------------------------------------------------------------------------
  const seen = front === 'checking' ? null : front.origin;
  return (
    <section className="ck-potato-card ck-potato-decision ck-potato-login" aria-label={`Sign-in from ${botName}`}>
      <p className="ck-potato-card-title">{text}</p>
      {card.why && <p className="ck-sub ck-potato-card-fact">{card.why}</p>}
      <p className="ck-sub ck-potato-login-front" role="status">
        {front === 'checking'
          ? LOGIN_COPY.checking
          : seen === null
            ? LOGIN_COPY.noPage
            : front.matches
              ? LOGIN_COPY.inFront(plainSite(seen))
              : LOGIN_COPY.mismatch(askedSite, plainSite(seen))}{' '}
        {front !== 'checking' && !front.matches && (
          <button type="button" className="ck-btn ck-btn--plain" disabled={busy} onClick={check}>
            {LOGIN_COPY.checkAgain}
          </button>
        )}
      </p>
      <div className="ck-potato-login-fields">
        <label className="ck-potato-login-field">
          <span className="ck-sub">{LOGIN_COPY.userName}</span>
          <input
            className="ck-potato-add-input"
            type="text"
            name="pm-login-user"
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            disabled={busy}
            value={username}
            onChange={(e) => setUsername(e.target.value)}
          />
        </label>
        <label className="ck-potato-login-field">
          <span className="ck-sub">{LOGIN_COPY.password}</span>
          <input
            ref={passwordBox}
            className="ck-potato-add-input"
            type="password"
            name="pm-login-secret"
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            disabled={busy}
            onInput={(e) => setBoxFilled(e.currentTarget.value !== '')}
          />
          <span className="ck-sub ck-potato-card-fact">{LOGIN_COPY.passwordWarning}</span>
        </label>
      </div>
      <p className="ck-sub ck-potato-card-fact">{LOGIN_COPY.warning}</p>
      <p className="ck-sub ck-potato-card-fact">{LOGIN_COPY.privacy}</p>
      {busy && (
        <p className="ck-sub ck-potato-card-fact" role="status">
          {LOGIN_COPY.signingIn}
        </p>
      )}
      {unclear && (
        <div className="ck-potato-login-unclear" role="status">
          <p className="ck-potato-card-title">{LOGIN_COPY.unclearTitle}</p>
          <p className="ck-sub ck-potato-card-fact">{LOGIN_COPY.unclearBody}</p>
          <div className="ck-potato-card-actions">
            <button
              type="button"
              className="ck-btn ck-btn--secondary"
              disabled={busy}
              onClick={() => {
                setUnclear(false);
                passwordBox.current?.focus();
              }}
            >
              {LOGIN_COPY.tryAgain}
            </button>
            <button type="button" className="ck-btn ck-btn--primary" disabled={busy} onClick={() => void run(onConfirm)}>
              {LOGIN_COPY.itWorked}
            </button>
            <button type="button" className="ck-btn ck-btn--plain" disabled={busy} onClick={() => void run(onNotNow)}>
              {LOGIN_COPY.choices[2]}
            </button>
          </div>
        </div>
      )}
      <div className="ck-potato-card-actions">
        <button type="button" className="ck-btn ck-btn--primary" disabled={!ready} onClick={() => void signIn()}>
          {LOGIN_COPY.choices[0]}
        </button>
        <button type="button" className="ck-btn ck-btn--secondary" disabled={busy} onClick={() => void run(onTakeOver)}>
          {LOGIN_COPY.choices[1]}
        </button>
        <button type="button" className="ck-btn ck-btn--plain" disabled={busy} onClick={() => void run(onNotNow)}>
          {LOGIN_COPY.choices[2]}
        </button>
      </div>
      {problem && (
        <p className="ck-sub ck-potato-card-problem" role="status">
          {problem}
        </p>
      )}
    </section>
  );
}
