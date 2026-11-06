// Settings > Potato Bot > Sandbox: the status, Set up, Start, Stop, Reset, Remove and the idle-stop minutes. The rows are shown only
// while Labs has the sandbox on (`labsFlag`); the Labs switch itself is a row of General > Labs (collected there by `labs`).
// Reset asks first; Remove needs the word "remove" typed; the Windows' Linux feature button opens a confirm dialog and nothing
// runs until its button is pressed.
import { useEffect, useRef, useState } from 'react';
import {
  LOOK_LABEL,
  OS_CHOICES,
  REMOVE_WORD,
  STATE_WORD,
  formatSize,
  osLabel,
  osRecipe,
  plainSandboxError,
  tauriSandboxTransport,
  type FeatureAsk,
  type Look,
  type SandboxTransport,
  type SandboxView,
  type VercelTools,
} from '../../../lib/potato-sandbox';
import '../../../styles/potato-sandbox.css';
import { ConfirmSheet, NumberControl } from '../controls';
import type { RowDef, SettingsEnv } from '../model';
import { Btn, Note, SettingBlock, SettingRow } from '../settings-ui';
import { LabsSwitch } from '../LabsSwitch';
import { InstallProgress, SandboxWizard } from './SandboxWizard';
import { useSandbox } from './useSandbox';

export const MIN_IDLE = 1;
export const MAX_IDLE = 240;

/** What the sandbox is, in the words under its name. */
export const SANDBOX_ABOUT = 'A small private Linux computer the bot can use, with its own browser and files. It cannot see your files, programs or accounts.';

/** The sheet that asks for the word "remove". The button stays off until it is typed. */
export function RemoveSheet({ busy, onConfirm, onCancel }: { busy?: boolean; onConfirm: () => void; onCancel: () => void }) {
  const [typed, setTyped] = useState('');
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);
  const ok = typed.trim().toLowerCase() === REMOVE_WORD;
  return (
    <div
      className="st-scrim"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onCancel();
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') onCancel();
      }}
    >
      <div className="st-sheet" role="dialog" aria-modal="true" aria-labelledby="sbx-remove-title">
        <h2 id="sbx-remove-title">Remove the sandbox?</h2>
        <p>Everything in it is deleted: its files, its sign-ins and its reset point. You can set it up again later. To go on, type the word {REMOVE_WORD}.</p>
        <input
          ref={input}
          className="st-field sbx-typed"
          aria-label={`Type ${REMOVE_WORD} to confirm`}
          value={typed}
          autoComplete="off"
          spellCheck={false}
          onChange={(e) => setTyped(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && ok && !busy) onConfirm();
          }}
        />
        <div className="st-sheet-actions">
          <Btn onClick={onCancel}>Cancel</Btn>
          <Btn primary disabled={!ok || busy} onClick={onConfirm}>
            Remove sandbox
          </Btn>
        </div>
      </div>
    </div>
  );
}

/** Pick an OS to add to the sandbox. The ones already installed are left out. */
export function AddOsSheet({ installed, busy, onAdd, onCancel }: { installed: string[]; busy?: boolean; onAdd: (id: Look) => void; onCancel: () => void }) {
  const taken = new Set(installed.map(osRecipe).filter(Boolean));
  const choices = OS_CHOICES.filter((o) => !taken.has(o.recipe));
  const [pick, setPick] = useState<Look | null>(choices[0]?.id ?? null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onCancel]);
  return (
    <div
      className="st-scrim"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onCancel();
      }}
    >
      <div className="st-sheet" role="dialog" aria-modal="true" aria-labelledby="sbx-add-os-title">
        <h2 id="sbx-add-os-title">Add another OS</h2>
        <p>It is added beside the one you have. The sandbox stops while it installs, then comes back in the new OS. You can switch back at any time.</p>
        {choices.length === 0 ? (
          <p className="sbx-hint">Every OS this version offers is already installed.</p>
        ) : (
          <div className="sbx-choices">
            {choices.map((o) => (
              <label key={o.id} className={`sbx-choice${pick === o.id ? ' is-on' : ''}`}>
                <input type="radio" name="sbx-add-os" checked={pick === o.id} onChange={() => setPick(o.id)} />
                <span className="sbx-choice-text">
                  <span className="sbx-choice-title">{LOOK_LABEL[o.id]}</span>
                  <small>{o.description}</small>
                </span>
              </label>
            ))}
          </div>
        )}
        <div className="st-sheet-actions">
          <Btn onClick={onCancel}>Cancel</Btn>
          <Btn primary disabled={!pick || busy || choices.length === 0} onClick={() => pick && onAdd(pick)}>
            {busy ? 'Adding…' : 'Add and switch'}
          </Btn>
        </div>
      </div>
    </div>
  );
}

const osRank = (id: string) => {
  const at = OS_CHOICES.findIndex((o) => o.id === id);
  return at < 0 ? OS_CHOICES.length : at;
};

function statusLine(view: SandboxView): string {
  const parts: string[] = [STATE_WORD[view.state]];
  if (view.reason && view.state !== 'installing' && view.state !== 'incomplete' && !view.restartPending) parts.push(view.reason);
  return parts.join('. ').replace(/\.\./g, '.');
}

function SandboxPanel({ transport, env }: { transport: SandboxTransport; env: SettingsEnv }) {
  const { view, failed, refresh } = useSandbox(transport);
  const [wizard, setWizard] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [addingOs, setAddingOs] = useState(false);
  // "Start over" is a remove (same typed word) that goes straight on to the wizard.
  const [startOver, setStartOver] = useState(false);
  const [ask, setAsk] = useState<FeatureAsk | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  const run = async (what: string, call: () => Promise<unknown>, done?: string) => {
    setBusy(what);
    setMessage(null);
    try {
      await call();
      if (done) setMessage({ ok: true, text: done });
    } catch (error) {
      setMessage({ ok: false, text: plainSandboxError(error, 'That did not work.') });
    } finally {
      setBusy(null);
      await refresh();
    }
  };

  const openAsk = async () => {
    setMessage(null);
    try {
      setAsk(await transport.linuxAsk());
    } catch (error) {
      setMessage({ ok: false, text: plainSandboxError(error, "Could not ask Windows about its Linux feature.") });
    }
  };

  const confirmAsk = async () => {
    const current = ask;
    setAsk(null);
    if (!current) return;
    await run(
      'linux',
      async () => {
        const done = await transport.linuxEnable(current.token);
        // A restart is said by the status itself ("Restart your PC to finish, then come back."), never beside "is on".
        setMessage(done.restartNeeded ? null : { ok: true, text: "Windows' Linux feature is on." });
      },
    );
  };

  const reset = async () => {
    const yes = await env.confirm({
      title: 'Reset the sandbox?',
      body: 'It goes back to how it was at the end of set up. Everything done in it since is lost, including sign-ins.',
      confirmLabel: 'Reset',
    });
    if (yes) await run('reset', () => transport.reset(), 'The sandbox is back to its reset point.');
  };

  const addOs = async (desktop: Look) => {
    setBusy('add-os');
    setMessage(null);
    try {
      await transport.addOs(desktop);
      setAddingOs(false);
    } catch (error) {
      setMessage({ ok: false, text: plainSandboxError(error, 'Could not add that OS.') });
    } finally {
      setBusy(null);
      await refresh();
    }
  };

  const switchOs = async (desktop: string) => {
    if (desktop === view?.activeDesktop) return;
    await run('switch-os', () => transport.switchOs(desktop as Look), `The sandbox now uses ${osLabel(desktop)}.`);
  };

  const remove = async () => {
    setBusy('remove');
    setMessage(null);
    const again = startOver;
    try {
      await transport.remove(REMOVE_WORD);
      setRemoving(false);
      setStartOver(false);
      setMessage({ ok: true, text: 'The sandbox was removed.' });
      if (again) setWizard(true);
    } catch (error) {
      setMessage({ ok: false, text: plainSandboxError(error, 'Could not remove the sandbox.') });
    } finally {
      setBusy(null);
      await refresh();
    }
  };

  if (!view) {
    return (
      <SettingBlock label="Sandbox" description={SANDBOX_ABOUT}>
        {failed ? <Note role="alert">{failed}</Note> : <span className="st-value" aria-busy="true">Loading…</span>}
      </SettingBlock>
    );
  }

  const installed = view.state === 'stopped' || view.state === 'running' || view.state === 'starting' || view.state === 'failed';
  const setting = view.state === 'installing';

  return (
    <SettingBlock label="Sandbox" description={SANDBOX_ABOUT}>
      <div className="sbx-status">
        <div className="sbx-status-line">
          <span className="sbx-state" data-state={view.state}>
            {statusLine(view)}
          </span>
          {installed && view.diskBytes !== null && <small>Uses {formatSize(view.diskBytes)} on this computer</small>}
          {installed && view.resetPointBytes !== null && <small>Reset point: {formatSize(view.resetPointBytes)}</small>}
        </div>

        {installed && view.desktops.length > 0 && (
          <div className="sbx-os" role="group" aria-label="Sandbox OS">
            <div className="sbx-os-head">
              <span className="sbx-os-label">OS</span>
              <small>{view.desktops.length > 1 ? 'Pick the desktop the bot sees. Switching restarts the screen.' : 'Add another OS and switch between them at any time.'}</small>
            </div>
            <div className="sbx-os-chips">
              {[...view.desktops].sort((a, b) => osRank(a) - osRank(b)).map((id) => (
                <button
                  key={id}
                  type="button"
                  className={`sbx-os-chip${id === view.activeDesktop ? ' is-active' : ''}`}
                  aria-pressed={id === view.activeDesktop}
                  disabled={busy !== null || setting}
                  onClick={() => void switchOs(id)}
                >
                  {osLabel(id)}
                  {id === view.activeDesktop && <small>Active</small>}
                </button>
              ))}
              <Btn disabled={busy !== null || setting} onClick={() => setAddingOs(true)}>
                Add OS…
              </Btn>
            </div>
            {busy === 'switch-os' && (
              <small className="sbx-hint" role="status">
                Switching the OS…
              </small>
            )}
          </div>
        )}

        {setting && (
          <>
            <InstallProgress install={view.install} />
            <div className="sbx-buttons">
              <Btn onClick={() => setWizard(true)}>Show</Btn>
              <Btn disabled={view.install.cancelling} onClick={() => void run('cancel', () => transport.cancelInstall())}>
                {view.install.cancelling ? 'Stopping…' : 'Stop set up'}
              </Btn>
            </div>
          </>
        )}

        {view.state === 'incomplete' && (
          <>
            <Note>
              Set up was cut off before it ended (the app was closed, or the computer slept). {view.canContinue ? 'Continue where it stopped, or start over.' : 'Start over to set it up again.'}
            </Note>
            <div className="sbx-buttons">
              {view.canContinue && (
                <Btn primary disabled={busy !== null} onClick={() => void run('continue', () => transport.continueInstall())}>
                  {busy === 'continue' ? 'Continuing…' : 'Continue setup'}
                </Btn>
              )}
              <Btn
                primary={!view.canContinue}
                disabled={busy !== null}
                onClick={() => {
                  setStartOver(true);
                  setRemoving(true);
                }}
              >
                Start over…
              </Btn>
            </div>
          </>
        )}

        {view.state === 'not_installed' && (
          <div className="sbx-buttons">
            {view.restartPending && <Note role="status">Restart your PC to finish, then come back.</Note>}
            {!view.linuxFeature && !view.restartPending && (
              <>
                <Note>Needs Windows' Linux feature (admin + restart).</Note>
                <Btn disabled={busy !== null} onClick={() => void openAsk()}>
                  Turn on Windows' Linux feature…
                </Btn>
              </>
            )}
            <Btn primary disabled={!view.linuxFeature} onClick={() => setWizard(true)}>
              Set up…
            </Btn>
          </div>
        )}

        {installed && (
          <div className="sbx-buttons">
            {view.state === 'running' ? (
              <Btn disabled={busy !== null} onClick={() => void run('stop', () => transport.stop())}>
                {busy === 'stop' ? 'Stopping…' : 'Stop'}
              </Btn>
            ) : (
              <Btn primary disabled={busy !== null || view.state === 'starting'} onClick={() => void run('start', () => transport.start())}>
                {busy === 'start' || view.state === 'starting' ? 'Starting…' : view.state === 'failed' ? 'Start again' : 'Start'}
              </Btn>
            )}
            <Btn disabled={busy !== null || !view.hasResetPoint} title={view.hasResetPoint ? undefined : 'No reset point was saved at set up.'} onClick={() => void reset()}>
              {busy === 'reset' ? 'Resetting…' : 'Reset…'}
            </Btn>
            <Btn disabled={busy !== null} onClick={() => setRemoving(true)}>
              Remove…
            </Btn>
          </div>
        )}

        {message && <Note role={message.ok ? 'status' : 'alert'}>{message.text}</Note>}
      </div>

      {wizard && (
        <SandboxWizard
          transport={transport}
          onClose={() => {
            setWizard(false);
            void refresh();
          }}
        />
      )}
      {removing && (
        <RemoveSheet
          busy={busy === 'remove'}
          onConfirm={() => void remove()}
          onCancel={() => {
            setRemoving(false);
            setStartOver(false);
          }}
        />
      )}
      {addingOs && (
        <AddOsSheet
          installed={view.desktops}
          busy={busy === 'add-os'}
          onAdd={(id) => void addOs(id)}
          onCancel={() => setAddingOs(false)}
        />
      )}
      {ask && <ConfirmSheet title={ask.title} body={ask.body} confirmLabel="Turn on" onConfirm={() => void confirmAsk()} onCancel={() => setAsk(null)} />}
    </SettingBlock>
  );
}

function IdleRow({ transport, env }: { transport: SandboxTransport; env: SettingsEnv }) {
  const { view } = useSandbox(transport, { poll: false });
  const [minutes, setMinutes] = useState<number | null>(null);
  const shown = minutes ?? view?.idleMinutes ?? 15;
  return (
    <SettingRow label="Stop when idle" description="The sandbox stops by itself after this many minutes of not being used. It starts again when a bot needs it.">
      <NumberControl
        aria-label="Stop the sandbox after minutes idle"
        value={shown}
        min={MIN_IDLE}
        max={MAX_IDLE}
        suffix="minutes"
        disabled={!view}
        onCommit={async (next) => {
          env.setError(null);
          try {
            setMinutes(await transport.setIdle(next));
            env.markSaved();
          } catch (error) {
            env.setError(plainSandboxError(error, 'Could not save that.'));
          }
        }}
      />
    </SettingRow>
  );
}

const KEYWORDS = 'sandbox linux wsl vm virtual computer desktop ubuntu mint cinnamon zorin chrome coding daily os switch';

/** Vercel's CLI and MCP bridge, installed inside the sandbox where the bot's terminal can reach them. */
function VercelRow({ transport, env }: { transport: SandboxTransport; env: SettingsEnv }) {
  const { view } = useSandbox(transport, { poll: false });
  const [tools, setTools] = useState<VercelTools | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const running = view?.state === 'running';
  useEffect(() => {
    // Only look while the sandbox is up: a check must never start it by itself.
    if (!running) return;
    let alive = true;
    transport
      .vercelStatus()
      .then((next) => {
        if (alive) setTools(next);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [transport, running]);
  const install = async () => {
    env.setError(null);
    setBusy(true);
    try {
      setTools(await transport.vercelInstall());
      setMessage('Installed in the sandbox. The bot can run vercel from its terminal.');
      env.markSaved();
    } catch (error) {
      env.setError(plainSandboxError(error, 'Could not install the Vercel tools.'));
    } finally {
      setBusy(false);
    }
  };
  return (
    <SettingRow
      label="Vercel tools"
      description="Installs the Vercel CLI and the mcp-remote bridge inside the sandbox, in the bot's own folder. The bot can then use them from run_terminal. Needs Node.js in the sandbox (the Coding set up includes it)."
    >
      {tools?.installed ? (
        <Note>{message ?? 'Installed. The bot can run vercel from its terminal.'}</Note>
      ) : (
        <>
          <Btn onClick={() => void install()} disabled={busy}>
            {busy ? 'Installing…' : 'Install in sandbox'}
          </Btn>
          {!running && <Note>Start the sandbox to check whether the tools are already installed.</Note>}
          {message && <Note>{message}</Note>}
        </>
      )}
    </SettingRow>
  );
}

export function makeSandboxRows(transport: SandboxTransport = tauriSandboxTransport): RowDef[] {
  return [
    {
      id: 'potato-sandbox-labs',
      label: 'Sandbox',
      hint: "A small private Linux computer a Potato Bot can use. Turn it on to see its settings under Potato Bot. Needs Potato Bot on too.",
      keywords: `${KEYWORDS} labs early`,
      group: 'Labs',
      labs: { name: 'Sandbox', about: "A small private Linux computer the bot can use, away from your files." },
      render: (env) => (
        <LabsSwitch
          label="Sandbox"
          description="Set up a private Linux computer inside Windows for Potato Bot. It cannot see your files. Its settings appear under Potato Bot when this is on. It does nothing until you set it up."
          checked={env.settings.potato_sandbox_enabled ?? false}
          onChange={(value) => void env.patch({ potato_sandbox_enabled: value })}
        />
      ),
    },
    {
      id: 'potato-sandbox',
      label: 'Sandbox',
      hint: 'Set up, start, stop, reset or remove the small Linux computer a bot can use.',
      keywords: `${KEYWORDS} set up install start stop reset remove wizard disk`,
      group: 'Sandbox',
      labsFlag: 'potato_sandbox_enabled',
      render: (env) => <SandboxPanel transport={transport} env={env} />,
    },
    {
      id: 'potato-sandbox-idle',
      label: 'Stop when idle',
      hint: 'How many minutes the sandbox may sit unused before it stops by itself.',
      keywords: `${KEYWORDS} idle minutes timeout`,
      group: 'Sandbox',
      labsFlag: 'potato_sandbox_enabled',
      render: (env) => <IdleRow transport={transport} env={env} />,
    },
    {
      id: 'potato-sandbox-vercel',
      label: 'Vercel tools',
      hint: 'Install the Vercel CLI and MCP bridge in the sandbox for the bot.',
      keywords: `${KEYWORDS} vercel mcp cli deploy terminal tools`,
      group: 'Sandbox',
      labsFlag: 'potato_sandbox_enabled',
      render: (env) => <VercelRow transport={transport} env={env} />,
    },
  ];
}
