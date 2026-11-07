import { lazy, StrictMode, Suspense } from 'react';
import ReactDOM from 'react-dom/client';
import CockpitApp from './CockpitApp';
import { isSettingsEntry } from './lib/settings-entry';
import './styles/index.css';

// The Settings and Terminal windows are separate chunks: the cockpit, which opens first, does not carry their code.
const SettingsApp = lazy(() => import('./SettingsApp'));
const TerminalApp = lazy(() => import('./TerminalApp'));

const params = new URLSearchParams(window.location.search);
const isTerminalApp = params.has('terminal') || params.has('cmd') || window.location.hash === '#terminal';

function resolveRoot() {
  if (isSettingsEntry(window.location.search)) return SettingsApp;
  if (params.get('cockpit') === '1') return CockpitApp;
  if (isTerminalApp) return TerminalApp;
  return CockpitApp;
}

const Root = resolveRoot();
ReactDOM.createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <Suspense fallback={null}>
      <Root />
    </Suspense>
  </StrictMode>,
);
