// Settings inside the cockpit's main area. It is the same SettingsPanel the standalone Settings
// window renders; this file only supplies the inline way out.
import { useEffect, useRef } from 'react';
import { SettingsPanel } from '../SettingsPanel';
import { useProjectPath } from '../../hooks/useProjectPath';
import type { SettingsPageId } from '../settings/model';

interface Props {
  initialTab?: SettingsPageId;
  initialRow?: string;
  onBack: () => void;
  onPageChange?: (page: SettingsPageId) => void;
}

function isEditing(target: EventTarget | null): target is HTMLElement {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true;
  if (!(target instanceof HTMLInputElement)) return false;
  return !['checkbox', 'radio', 'button', 'submit', 'range'].includes(target.type);
}

export function SettingsView({ initialTab, initialRow, onBack, onPageChange }: Props) {
  const { projectPath, setProjectPath } = useProjectPath();
  const host = useRef<HTMLDivElement>(null);
  const backRef = useRef(onBack);
  backRef.current = onBack;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented || e.isComposing) return;
      if (host.current?.querySelector('[role="dialog"]') || document.querySelector('.ck-scrim')) return;
      const target = e.target;
      if (target instanceof HTMLInputElement && target.type === 'search' && target.value !== '') return;
      if (isEditing(target)) {
        target.blur();
        return;
      }
      e.preventDefault();
      backRef.current();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  return (
    <div className="ck-settings-host" ref={host}>
      <SettingsPanel
        open
        inline
        initialTab={initialTab}
        initialRow={initialRow}
        onPageChange={onPageChange}
        projectPath={projectPath}
        onProjectPathChange={setProjectPath}
        onClose={onBack}
      />
    </div>
  );
}
