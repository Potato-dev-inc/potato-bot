// Standalone Settings window (label "settings"): the Settings panel rendered full-window.
import { useMemo, useRef } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { SettingsPanel } from './components/SettingsPanel';
import type { SettingsPageId } from './components/settings/model';
import { useProjectPath } from './hooks/useProjectPath';
import { settingsEntryTarget } from './lib/settings-target';

export default function SettingsApp() {
  const target = useMemo(() => settingsEntryTarget(window.location.search), []);
  const { projectPath, setProjectPath } = useProjectPath();
  const page = useRef<SettingsPageId>(target.page);

  return (
    <SettingsPanel
      open
      initialTab={target.page}
      initialRow={target.row}
      onPageChange={(p) => {
        page.current = p;
      }}
      projectPath={projectPath}
      onProjectPathChange={setProjectPath}
      onClose={() => {
        void getCurrentWindow().close();
      }}
    />
  );
}
