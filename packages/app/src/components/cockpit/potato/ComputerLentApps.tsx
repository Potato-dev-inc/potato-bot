// "Apps from your PC" under the Computer tab's upkeep card (Labs `potato_lend_app_enabled`, Windows only). Each lent app
// is a row: its name, who has it, Show it / Keep it out of my way (one segmented control), Take control or Hand back,
// and Bring back. The list is read from the app's commands and read again on `potato://lend`. Errors are shown inline in
// plain words.
import { useCallback, useEffect, useRef, useState } from 'react';
import { LEND_COPY } from '../../../lib/potato-computer';
import { lendErrorText, tauriLendTransport, type LendMode, type LendTransport, type LentView } from '../../../lib/potatoLend';
import { LendPicker } from './LendPicker';

export interface ComputerLentAppsProps {
  bot: { name: string };
  transport?: LendTransport;
}

/** How long the "is yours again" line stays in place of a row after Bring back. */
export const RETURNED_MS = 3000;

export function ComputerLentApps({ bot, transport = tauriLendTransport }: ComputerLentAppsProps) {
  const [apps, setApps] = useState<LentView[]>([]);
  const [returned, setReturned] = useState<{ id: string; name: string }[]>([]);
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());

  const refresh = useCallback(async () => {
    try {
      setApps(await transport.list());
    } catch (e) {
      setError(lendErrorText(e));
    }
  }, [transport]);

  useEffect(() => {
    void refresh();
    let live = true;
    let unlisten: (() => void) | null = null;
    const subscribed = transport.onChanged?.(() => void refresh());
    subscribed?.then(
      (off) => {
        if (live) unlisten = off;
        else off();
      },
      () => undefined,
    );
    return () => {
      live = false;
      unlisten?.();
    };
  }, [transport, refresh]);

  // Pending "is yours again" timers are dropped when the card goes away.
  useEffect(() => {
    const pending = timers.current;
    return () => {
      pending.forEach((timer) => clearTimeout(timer));
      pending.clear();
    };
  }, []);

  // One action at a time: run it, show a failure inline, and read the list again either way.
  const act = async (run: () => Promise<unknown>) => {
    setError(null);
    try {
      await run();
    } catch (e) {
      setError(lendErrorText(e));
    }
    await refresh();
  };

  // Bring back: on success the row is replaced by "{App} is yours again." for a few seconds, then it is gone.
  const bringBack = async (app: LentView) => {
    setError(null);
    try {
      await transport.bringBack(app.id);
      const id = app.id;
      setReturned((list) => [...list.filter((r) => r.id !== id), { id, name: app.appName }]);
      const old = timers.current.get(id);
      if (old) clearTimeout(old);
      timers.current.set(
        id,
        setTimeout(() => {
          timers.current.delete(id);
          setReturned((list) => list.filter((r) => r.id !== id));
        }, RETURNED_MS),
      );
    } catch (e) {
      setError(lendErrorText(e));
    }
    await refresh();
  };

  const isReturned = (id: string) => returned.some((r) => r.id === id);
  const returnedNotListed = returned.filter((r) => !apps.some((a) => a.id === r.id));

  return (
    <section className="ck-potato-lend" aria-label={LEND_COPY.title}>
      <h3 className="ck-potato-lend-title">{LEND_COPY.title}</h3>
      {apps.length === 0 && returned.length === 0 ? (
        <>
          <p className="ck-sub ck-potato-lend-empty">{LEND_COPY.empty(bot.name)}</p>
          <p className="ck-sub ck-potato-lend-empty">{LEND_COPY.emptyHint}</p>
        </>
      ) : (
        <ul className="ck-potato-lend-rows">
          {apps.map((app) =>
            isReturned(app.id) ? (
              <ReturnedRow key={app.id} name={app.appName} />
            ) : (
              <LentRow
                key={app.id}
                app={app}
                botName={bot.name}
                onMode={(mode) => void act(() => transport.setMode(app.id, mode))}
                onTake={() => void act(() => transport.takeOver(app.id))}
                onHand={() => void act(() => transport.handBack(app.id))}
                onBring={() => void bringBack(app)}
              />
            ),
          )}
          {returnedNotListed.map((r) => (
            <ReturnedRow key={r.id} name={r.name} />
          ))}
        </ul>
      )}
      <div className="ck-potato-maint-buttons">
        <button type="button" className="ck-btn ck-btn--secondary" onClick={() => setPicking(true)}>
          {LEND_COPY.lendButton}
        </button>
      </div>
      {error && (
        <p role="alert" className="ck-potato-lend-error">
          {error}
        </p>
      )}
      {picking && (
        <LendPicker
          botName={bot.name}
          transport={transport}
          onClose={() => setPicking(false)}
          onLent={() => {
            setPicking(false);
            void refresh();
          }}
        />
      )}
    </section>
  );
}

function stateText(app: LentView, botName: string): string {
  switch (app.phase) {
    case 'bot':
      return LEND_COPY.stateBot(botName);
    case 'you':
      return LEND_COPY.stateYou(botName);
    case 'needs_you':
      return LEND_COPY.stateNeedsYou(botName, app.reason ?? null);
    default:
      return LEND_COPY.stateFree(botName);
  }
}

function ReturnedRow({ name }: { name: string }) {
  return (
    <li className="ck-potato-lend-row">
      <span className="ck-potato-lend-icon" aria-hidden="true">
        {name.charAt(0).toUpperCase()}
      </span>
      <div className="ck-potato-lend-main">
        <p className="ck-sub ck-potato-lend-state" role="status">
          {LEND_COPY.returned(name)}
        </p>
      </div>
    </li>
  );
}

function LentRow({
  app,
  botName,
  onMode,
  onTake,
  onHand,
  onBring,
}: {
  app: LentView;
  botName: string;
  onMode: (mode: LendMode) => void;
  onTake: () => void;
  onHand: () => void;
  onBring: () => void;
}) {
  const modes: { mode: LendMode; label: string; hint: string }[] = [
    { mode: 'visible', label: LEND_COPY.visible, hint: LEND_COPY.visibleHint },
    { mode: 'hidden', label: LEND_COPY.hidden, hint: LEND_COPY.hiddenHint },
  ];
  return (
    <li className="ck-potato-lend-row">
      <span className="ck-potato-lend-icon" aria-hidden="true">
        {app.appName.charAt(0).toUpperCase()}
      </span>
      <div className="ck-potato-lend-main">
        <p className="ck-potato-lend-name">{app.appName}</p>
        <p className="ck-sub ck-potato-lend-state">
          {stateText(app, botName)}
          {app.pictureLive === false && ` · ${LEND_COPY.picturePaused}`}
        </p>
      </div>
      <div className="ck-potato-lend-actions">
        <div className="ck-potato-lend-seg" role="group" aria-label={LEND_COPY.modeLabel}>
          {modes.map(({ mode, label, hint }) => (
            <button
              key={mode}
              type="button"
              className="ck-btn ck-btn--plain"
              title={hint}
              aria-pressed={app.mode === mode}
              onClick={() => {
                if (app.mode !== mode) onMode(mode);
              }}
            >
              {label}
            </button>
          ))}
        </div>
        {app.phase === 'you' ? (
          <button type="button" className="ck-btn ck-btn--secondary" onClick={onHand}>
            {LEND_COPY.handBack}
          </button>
        ) : (
          <button type="button" className="ck-btn ck-btn--secondary" onClick={onTake}>
            {LEND_COPY.takeControl}
          </button>
        )}
        <button type="button" className="ck-btn ck-btn--plain" onClick={onBring}>
          {LEND_COPY.bringBack}
        </button>
      </div>
    </li>
  );
}
