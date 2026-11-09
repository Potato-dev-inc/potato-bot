// Settings > Potato Bot > Labs: the switch for Spaces (Labs `spaces_enabled`, off by default), and the name people see in Spaces.
import { useEffect, useState } from 'react';
import type { RowDef, SettingsEnv } from '../model';
import { LabsSwitch } from '../LabsSwitch';
import { TextControl } from '../controls';
import { SettingRow } from '../settings-ui';
import { spacesDisplayName, spacesErrorText, spacesSetDisplayName } from '../../../lib/spaces';

/** "Display name": the one name every space shows for this computer's person. Saved on Enter or when the field loses focus. */
export function SpacesNameRow({ env }: { env: SettingsEnv }) {
  const [name, setName] = useState('');
  useEffect(() => {
    let alive = true;
    spacesDisplayName()
      .then((r) => { if (alive) setName(r.displayName ?? ''); })
      .catch(() => undefined);
    return () => { alive = false; };
  }, []);
  return (
    <SettingRow label="Display name" description="The name people see for you in Spaces. A change reaches every space you are in.">
      <TextControl
        value={name}
        mode="blur"
        maxLength={40}
        placeholder="Your name"
        aria-label="Display name"
        normalize={(s) => s.trim()}
        onCommit={async (next) => {
          if (next === '') return;
          try {
            const r = await spacesSetDisplayName(next);
            setName(r.displayName);
            env.setError(null);
            env.markSaved();
          } catch (e) {
            env.setError(spacesErrorText(e));
          }
        }}
      />
    </SettingRow>
  );
}

export const spacesRows: RowDef[] = [
  {
    id: 'spaces-labs',
    label: 'Spaces',
    hint: 'Chat with friends in small groups, computer to computer. No server keeps your messages.',
    keywords: 'spaces chat friends groups small group computer to computer labs early',
    group: 'Labs',
    labs: { name: 'Spaces', about: 'Chat with friends in small groups, computer to computer.' },
    render: (env) => (
      <LabsSwitch
        label="Spaces"
        description="Chat with friends in small groups, computer to computer. No server keeps your messages."
        checked={env.settings.spaces_enabled ?? false}
        onChange={(value) => void env.patch({ spaces_enabled: value })}
      />
    ),
  },
  {
    id: 'spaces-display-name',
    label: 'Display name',
    hint: 'The name people see for you in Spaces.',
    keywords: 'spaces name display nickname rename who people see',
    group: 'Spaces',
    labsFlag: 'spaces_enabled',
    render: (env) => <SpacesNameRow env={env} />,
  },
];
