// Settings > Potato Bot > Labs: the switch for Spaces (Labs `spaces_enabled`, off by default).
import type { RowDef } from '../model';
import { LabsSwitch } from '../LabsSwitch';

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
];
