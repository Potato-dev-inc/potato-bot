// "Apps from your PC" under the Computer tab's upkeep card (Labs `potato_lend_app_enabled`, Windows only). Each lent app
// is a row: its name, who has it, Visible / Hidden, Take control or Hand back, and Bring back. The list is read from the
// app's commands and read again on `potato://lend`. Errors are shown inline in plain words.
import { useCallback, useEffect, useState } from 'react';
import { LEND_COPY } from '../../../lib/potato-computer';
import { lendErrorText, tauriLendTransport, type LendMode, type LendTransport, type LentView } from '../../../lib/potatoLend';
import { LendPicker } from './LendPicker';

export interface ComputerLentAppsProps {
  bot: { name: string };
  transport?: LendTransport;
}

export function ComputerLentApps({ bot, transport = tauriLendTransport }: ComputerLentAppsProps) {
  const [apps, setApps] = useState<LentView[]>([]);
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  return (
    <section className="ck-potato-lend" aria-label={LEND_COPY.title}>
      <h3 className="ck-potato-lend-title">{LEND_COPY.title}</h3>
      {apps.length === 0 ? (
        <p className="ck-sub ck-potato-lend-empty">{LEND_COPY.empty(bot.name)}</p>
      ) : (
        <ul className="ck-potato-lend-rows">
          {apps.map((app) => (
            <LentRow
              key={app.id}
              app={app}
              botName={bot.name}
              onMode={(mode) => void act(() => transport.setMode(app.id, mode))}
              onTake={() => void act(() => transport.takeOver(app.id))}
              onHand={() => void act(() => transport.handBack(app.id))}
              onBring={() => void act(() => transport.bringBack(app.id))}
            />
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
      return LEND_COPY.stateYou;
    case 'needs_you':
      return LEND_COPY.stateNeedsYou(botName, app.reason ?? null);
    default:
      return LEND_COPY.stateFree;
  }
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
          {(['visible', 'hidden'] as const).map((mode) => (
            <button
              key={mode}
              type="button"
              className="ck-btn ck-btn--plain"
              aria-pressed={app.mode === mode}
              onClick={() => {
                if (app.mode !== mode) onMode(mode);
              }}
            >
              {mode === 'visible' ? LEND_COPY.visible : LEND_COPY.hidden}
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
