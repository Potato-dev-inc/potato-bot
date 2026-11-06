// Standalone pane window (`index.html?terminal=1&pane=<id>`): one real terminal from the PTY registry.
import { useEffect, useState } from 'react';
import { usePaneRegistry } from './hooks/usePaneRegistry';
import { TerminalViewport } from './components/TerminalViewport';
import { applyTheme, loadSettings, type AppTheme } from './lib/settings';
import { subscribeSettingsTheme } from './lib/settings-sync';

export default function TerminalApp() {
  const registry = usePaneRegistry();
  const params = new URLSearchParams(window.location.search);
  const paneId = params.get('pane');
  const pane = paneId ? registry.panes.get(paneId) : undefined;
  const [theme, setTheme] = useState<AppTheme>('dark');

  useEffect(() => {
    let live = true;
    loadSettings()
      .then((s) => {
        if (!live) return;
        setTheme(s.theme ?? 'dark');
        applyTheme(s.theme ?? 'dark');
      })
      .catch(() => {});
    const off = subscribeSettingsTheme((next) => {
      setTheme(next);
      applyTheme(next);
    });
    return () => {
      live = false;
      off();
    };
  }, []);

  return (
    <div className="terminal-app" data-theme={theme}>
      {pane ? (
        <TerminalViewport pane={pane} registry={registry} chrome />
      ) : (
        <div className="pm-terminal-empty">{registry.initialReady ? 'This pane is closed.' : 'Opening terminal...'}</div>
      )}
    </div>
  );
}
