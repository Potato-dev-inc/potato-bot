// Settings: "Check for updates". The same flow as the notice, with a button that says what it found.
import { tauriUpdateTransport, type UpdateTransport } from '../../../lib/app-update';
import { UpdateNoticeView } from '../../updates/UpdateNotice';
import { useAppUpdate } from '../../updates/useAppUpdate';
import { Btn, Note, SettingRow } from '../settings-ui';
import type { RowDef } from '../model';

export function UpdateRow({ transport = tauriUpdateTransport }: { transport?: UpdateTransport }) {
  const state = useAppUpdate(transport, false);
  const checking = state.phase === 'checking';
  return (
    <SettingRow
      label="Updates"
      description="Potato Bot looks for a new version now and then. Nothing installs until you press Update. Your bots, settings and the bot's computer stay as they are."
    >
      <div className="app-update-row">
        <Btn disabled={checking || state.phase === 'installing'} onClick={() => void state.check(true)}>
          {checking ? 'Checking…' : 'Check for updates'}
        </Btn>
        {state.phase === 'none' && <Note>You have the newest version.</Note>}
        <UpdateNoticeView state={state} />
      </div>
    </SettingRow>
  );
}

export function makeUpdateRows(transport: UpdateTransport = tauriUpdateTransport): RowDef[] {
  return [
    {
      id: 'potato-updates',
      label: 'Updates',
      hint: 'Check for a new version of Potato Bot and install it in one click.',
      keywords: 'update upgrade version new release install check',
      group: 'Potato Bot',
      render: () => <UpdateRow transport={transport} />,
    },
  ];
}
