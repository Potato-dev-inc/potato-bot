// Cockpit window (`index.html?cockpit=1`): the Potato Bot app.
import { lazy, Suspense, useEffect, useState } from 'react';
import { useCockpit } from './hooks/useCockpit';
import { useCompanions } from './hooks/useCompanions';
import { usePotatoState, readOpenBot, writeOpenBot } from './hooks/usePotato';
import { useGroupsOn } from './components/cockpit/potato/useGroupsOn';
import { cockpitApi } from './lib/cockpit-api';
import { tauriCompanionTransport } from './lib/companion';
import { tauriPotatoTransport } from './lib/potato';
import { tauriScreenTransport } from './lib/potatoScreen';
import { tauriSandboxTransport } from './lib/potato-sandbox';
import { registerSettingsOpener } from './lib/openSettingsAt';
import { resolveSettingsTab } from './lib/settings-entry';
import type { SettingsPageId } from './components/settings/model';
import { PotatoPage } from './components/cockpit/potato/PotatoPage';
import { totalUnread } from './components/cockpit/potato/model';
import { PotatoPaneView } from './components/cockpit/PotatoPaneView';
import { AppUpdateHost } from './components/updates/AppUpdateHost';
import { SpacesRow } from './components/cockpit/SpacesRow';
import { useSpacesEnabled } from './lib/spaces';
import './styles/cockpit.css';

// Settings is opened rarely: its own chunk keeps the first load small.
const SettingsView = lazy(() => import('./components/cockpit/SettingsView').then((m) => ({ default: m.SettingsView })));
// Spaces is behind a Labs flag and opened on demand: its own chunk keeps the main bundle under the size warning.
const SpacesPage = lazy(() => import('./components/spaces/SpacesPage').then((m) => ({ default: m.SpacesPage })));

type View = 'potato' | 'settings' | 'pane' | 'spaces';

function initialView(): View {
  return new URLSearchParams(window.location.search).get('view') === 'settings' ? 'settings' : 'potato';
}

function initialTab(): SettingsPageId {
  return resolveSettingsTab(new URLSearchParams(window.location.search).get('tab')) ?? 'potato';
}

export default function CockpitApp() {
  const c = useCockpit();
  const companions = useCompanions();
  const potato = usePotatoState();
  const [view, setView] = useState<View>(initialView);
  const [settingsTab, setSettingsTab] = useState<SettingsPageId>(initialTab);
  const [paneId, setPaneId] = useState<string | null>(null);
  const [potatoBotId, setPotatoBotId] = useState<string | null>(readOpenBot);
  const groupsOn = useGroupsOn();
  const spacesOn = useSpacesEnabled();

  const potatoOn = potato.state?.enabled === true;
  const openPotatoBot = (id: string) => {
    setPotatoBotId(id);
    writeOpenBot(id);
  };

  const openSettings = async (tab?: string) => {
    setSettingsTab(resolveSettingsTab(tab) ?? 'potato');
    setView('settings');
  };
  const nav = { ...c, openSettings };

  useEffect(() => {
    registerSettingsOpener((tab) => {
      setSettingsTab(resolveSettingsTab(tab) ?? 'potato');
      setView('settings');
    });
    return () => registerSettingsOpener(null);
  }, []);

  useEffect(() => {
    let disposed = false;
    let off: (() => void) | undefined;
    void cockpitApi
      .onView((e) => {
        if (e.view === 'potato') {
          if (e.bot) openPotatoBot(e.bot);
          setView('potato');
        }
      })
      .then((u) => (disposed ? u() : (off = u)));
    return () => {
      disposed = true;
      off?.();
    };
  }, []);

  const pane = paneId ? c.registry.panes.get(paneId) : undefined;
  if (view === 'pane' && paneId && pane) {
    return (
      <div className="cockpit" data-theme={c.theme}>
        <main className="ck-main">
          <PotatoPaneView
            c={nav}
            paneId={paneId}
            onBack={() => {
              setPaneId(null);
              setView('potato');
            }}
          />
        </main>
        <Toasts toasts={c.toasts} />
      </div>
    );
  }

  return (
    <div className="cockpit" data-theme={c.theme}>
      <aside className="ck-sidebar">
        <button
          type="button"
          className={`ck-row ck-row--flat${view === 'potato' ? ' is-active' : ''}`}
          aria-current={view === 'potato' ? 'true' : undefined}
          onClick={() => setView('potato')}
        >
          <span className="ck-row-glyph ck-row-glyph--icon ck-row-glyph--potato">P</span>
          <span className="ck-row-main">
            <span className="ck-row-title">Potato Bot</span>
          </span>
          {potato.state && totalUnread(potato.state.bots) > 0 && <span className="ck-count">{totalUnread(potato.state.bots)}</span>}
        </button>
        <SpacesRow enabled={spacesOn} open={view === 'spaces' && spacesOn} onOpen={() => setView('spaces')} />
        <button
          type="button"
          className={`ck-row ck-row--flat${view === 'settings' ? ' is-active' : ''}`}
          aria-current={view === 'settings' ? 'true' : undefined}
          onClick={() => void openSettings('potato')}
        >
          <span className="ck-row-glyph ck-row-glyph--icon">S</span>
          <span className="ck-row-main">
            <span className="ck-row-title">Settings</span>
          </span>
        </button>
      </aside>
      <main className="ck-main">
        {view === 'settings' ? (
          <Suspense fallback={null}>
            <SettingsView
              initialTab={settingsTab}
              onBack={() => setView('potato')}
              onPageChange={setSettingsTab}
            />
          </Suspense>
        ) : view === 'spaces' && spacesOn ? (
          <Suspense fallback={null}>
            <SpacesPage />
          </Suspense>
        ) : potato.state && potatoOn ? (
          <PotatoPage
            state={potato.state}
            version={potato.version}
            transport={tauriPotatoTransport}
            companionTransport={tauriCompanionTransport}
            companions={companions.companions}
            openBotId={potatoBotId}
            groupsOn={groupsOn}
            onOpenBot={openPotatoBot}
            onChanged={() => {
              potato.refresh();
              companions.refresh();
            }}
            onCompanionsChanged={companions.refresh}
            panes={c.registry.paneList.map((p) => ({
              id: p.info.id,
              agentType: p.info.agent_type,
              title: p.info.title ?? null,
              lane: p.info.lane ?? null,
            }))}
            onOpenPane={(id) => {
              setPaneId(id);
              setView('pane');
            }}
            paneExists={(id) => c.registry.panes.has(id)}
            onNotify={c.notify}
            onOpenSettings={() => void openSettings('potato')}
            screenTransport={tauriScreenTransport}
            sandboxTransport={tauriSandboxTransport}
            cockpit={nav}
          />
        ) : potato.state ? (
          <div className="ck-spawn ck-spawn--plain">
            <h1 className="ck-title">Potato Bot is off</h1>
            <p className="ck-sub">Turn it on in Settings, then come back here.</p>
            <button type="button" className="ck-btn ck-btn--secondary" onClick={() => void openSettings('potato')}>
              Open Settings
            </button>
          </div>
        ) : (
          <div className="ck-spawn ck-spawn--plain">
            <h1 className="ck-title">Potato Bot</h1>
            <p className="ck-sub">Starting up...</p>
          </div>
        )}
      </main>
      <Toasts toasts={c.toasts} />
      <AppUpdateHost />
    </div>
  );
}

function Toasts({ toasts }: { toasts: { id: number; text: string; action?: { label: string; run: () => void } }[] }) {
  return (
    <div className="ck-toasts" aria-live="polite">
      {toasts.map((t) => (
        <div className="ck-toast" key={t.id}>
          <span>{t.text}</span>
          {t.action && (
            <button type="button" className="ck-toast-action" onClick={t.action.run}>
              {t.action.label}
            </button>
          )}
        </div>
      ))}
    </div>
  );
}
