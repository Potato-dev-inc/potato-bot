import { StrictMode } from 'react';
import ReactDOM from 'react-dom/client';
import CockpitApp from './CockpitApp';
import SettingsApp from './SettingsApp';
import TerminalApp from './TerminalApp';
import { isSettingsEntry } from './lib/settings-entry';
import './styles/index.css';

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
    <Root />
  </StrictMode>,
);
