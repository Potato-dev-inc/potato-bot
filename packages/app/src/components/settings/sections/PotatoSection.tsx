// Settings > Potato Bot (spec section 5). It is shown only while Labs has Potato Bot on. The switch here is the kill
// switch (the same flag as General > Labs). Everything else is saved by Tauri commands in
// `<app data>/potato/settings.json`, never in the settings file the phone can patch. The DeepSeek key goes to the
// system keychain through the same key field as the other keys; this page only ever sees "set" and the last 4.
import { useState } from 'react';
import { dollars, PERSONA_LABEL, plainError, tauriPotatoTransport, type PotatoTransport, type TalkPersona } from '../../../lib/potato';
import { tauriUsageTransport, type PotatoUsageTransport } from '../../../lib/potato-usage';
import { tauriMcpTransport, type McpTransport } from '../../../lib/potato-mcp';
import { secretsApi, type SecretsApi } from '../../../lib/secrets';
import { ApiKeyField } from '../ApiKeyField';
import { NumberControl, Switch } from '../controls';
import type { RowDef } from '../model';
import { Btn, FieldSelect, Note, SettingBlock, SettingRow } from '../settings-ui';
import { LabsSwitch } from '../LabsSwitch';
import { makeSandboxRows } from '../sandbox/SandboxRows';
import { makePotatoVoiceRows } from './PotatoVoiceRows';
import { makeUpdateRows } from './UpdateRow';
import { makePotatoSubPageRows } from '../potato/PotatoSubPages';
import { RecentlyDeletedRow } from '../potato/RecentlyDeleted';
import { GoKeyField } from '../potato/GoKeyField';
import { usePotatoSettings } from '../potato/usePotatoSettings';

/** The limits of the daily limit, in cents ($0.05 to $20). Rust clamps again. */
export const MIN_LIMIT_CENTS = 5;
export const MAX_LIMIT_CENTS = 2000;
const PERSONAS: TalkPersona[] = ['fish', 'potato', 'pro'];

function KeyRow({ transport, api }: { transport: PotatoTransport; api: SecretsApi }) {
  const { settings } = usePotatoSettings(transport);
  const [checking, setChecking] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const check = async () => {
    setChecking(true);
    setResult(null);
    try {
      setResult({ ok: true, text: await transport.checkKey() });
    } catch (error) {
      setResult({ ok: false, text: plainError(error, 'Could not check the key.') });
    } finally {
      setChecking(false);
    }
  };
  if (settings && settings.brain !== 'deepseek') {
    return <SettingRow label="DeepSeek key" description={settings.brain === 'go' ? 'Not needed: this brain uses your OpenCode Go key.' : 'Not needed: this brain uses your own agents.'} />;
  }
  return (
    <SettingBlock label="DeepSeek key" description="Kept in your system keychain. Only the last 4 characters are ever shown here, and the key never goes to the phone.">
      <ApiKeyField
        label="DeepSeek key"
        name="deepseek_api_key"
        placeholder="sk-…"
        api={api}
        onSaved={() => setResult(null)}
        onFileChange={() => Promise.reject(new Error('Potato Bot keeps its key only in the system keychain, and this computer has none.'))}
      />
      <div className="st-key-check">
        <Btn disabled={checking} onClick={() => void check()}>
          {checking ? 'Checking…' : 'Check key'}
        </Btn>
        {result && (
          <Note role={result.ok ? 'status' : 'alert'}>{result.text}</Note>
        )}
      </div>
    </SettingBlock>
  );
}

function PersonaRow({ transport, onSaved, setError }: { transport: PotatoTransport; onSaved: () => void; setError: (m: string | null) => void }) {
  const { settings, save } = usePotatoSettings(transport);
  return (
    <SettingRow label="Default persona" description="The voice a new bot starts with. You can change a bot's voice any time in its chat.">
      <FieldSelect
        aria-label="Default persona"
        value={settings?.defaultPersona ?? 'fish'}
        disabled={!settings}
        onChange={(e) => {
          setError(null);
          save({ defaultPersona: e.target.value as TalkPersona }).then(onSaved, (error) => setError(plainError(error, 'Could not save that.')));
        }}
      >
        {PERSONAS.map((pack) => (
          <option key={pack} value={pack}>
            {PERSONA_LABEL[pack]}
          </option>
        ))}
      </FieldSelect>
    </SettingRow>
  );
}

/** Dollars typed in the box, kept between $0.05 and $20 and to whole cents. */
export function limitCentsOf(dollarsTyped: number): number {
  return Math.min(MAX_LIMIT_CENTS, Math.max(MIN_LIMIT_CENTS, Math.round(dollarsTyped * 100)));
}

function LimitRow({ transport, onSaved, setError }: { transport: PotatoTransport; onSaved: () => void; setError: (m: string | null) => void }) {
  const { settings, save } = usePotatoSettings(transport);
  return (
    <SettingRow
      label="Daily limit"
      description={`The most one bot may spend in a day, on every chat. From ${dollars(MIN_LIMIT_CENTS)} to ${dollars(MAX_LIMIT_CENTS)}. At the limit it stops until tomorrow.`}
    >
      <NumberControl
        aria-label="Daily limit in dollars"
        value={(settings?.dailyLimitCents ?? 50) / 100}
        min={MIN_LIMIT_CENTS / 100}
        max={MAX_LIMIT_CENTS / 100}
        step={0.05}
        normalize={(n) => limitCentsOf(n) / 100}
        suffix="dollars a day"
        disabled={!settings || settings.dailyLimitOff === true}
        onCommit={(next) => {
          setError(null);
          save({ dailyLimitCents: limitCentsOf(next) }).then(onSaved, (error) => setError(plainError(error, 'Could not save that.')));
        }}
      />
    </SettingRow>
  );
}

/** The plain note beside the No limit switch. */
export const NO_LIMIT_NOTE = "Spending won't stop by itself.";

function NoLimitRow({ transport, onSaved, setError }: { transport: PotatoTransport; onSaved: () => void; setError: (m: string | null) => void }) {
  const { settings, save } = usePotatoSettings(transport);
  const off = settings?.dailyLimitOff === true;
  return (
    <SettingRow label="No limit" description={NO_LIMIT_NOTE}>
      <Switch
        label="No limit"
        checked={off}
        disabled={!settings}
        onChange={(next) => {
          setError(null);
          save({ dailyLimitOff: next }).then(onSaved, (error) => setError(plainError(error, 'Could not save that.')));
        }}
      />
    </SettingRow>
  );
}

const KEYWORDS = 'potato fish bot chat friend brain deepseek opencode go agents persona limit budget cost';

export function makePotatoRows(transport: PotatoTransport = tauriPotatoTransport, api: SecretsApi = secretsApi, usage: PotatoUsageTransport = tauriUsageTransport, mcp: McpTransport = tauriMcpTransport): RowDef[] {
  const [models, usageRow, connectorsRow, mcpRow] = makePotatoSubPageRows(transport, usage, mcp);
  return [
    {
      id: 'potato-bot',
      label: 'Potato Bot',
      hint: 'The on and off switch. Off stops every bot brain now and takes back any helper request.',
      keywords: `${KEYWORDS} kill switch off on stop`,
      group: 'Potato Bot',
      render: (env) => (
        <LabsSwitch
          label="Potato Bot"
          description="On: your bots can chat and ask you to start helpers. Off: every brain stops now and open helper requests are taken back. Turn it back on in General › Labs."
          checked={env.settings.potato_bot_enabled ?? false}
          onChange={(value) => void env.patch({ potato_bot_enabled: value })}
        />
      ),
    },
    {
      id: 'potato-groups-labs',
      label: 'Bot groups',
      hint: 'Make a group of 2 to 10 of your bots and talk to them in one chat. Needs Potato Bot on too.',
      keywords: `${KEYWORDS} groups group rooms chat labs early`,
      group: 'Labs',
      labs: { name: 'Bot groups', about: 'Chat with 2 to 10 of your bots in one group. Needs Potato Bot.' },
      render: (env) => (
        <LabsSwitch
          label="Bot groups"
          description="On: you can make a group of 2 to 10 of your bots. Off: groups are hidden and nothing in them runs. Needs Potato Bot on too."
          checked={env.settings.potato_groups_enabled ?? false}
          onChange={(value) => void env.patch({ potato_groups_enabled: value })}
        />
      ),
    },
    models,
    {
      id: 'potato-recently-deleted',
      label: 'Recently deleted',
      hint: 'Notes, routines and bots you deleted in the last 7 days. Bring them back here.',
      keywords: 'potato recently deleted trash undo restore bring back bin 7 days delete',
      group: 'Potato Bot',
      render: () => <RecentlyDeletedRow />,
    },
    {
      id: 'potato-key',
      label: 'DeepSeek key',
      hint: 'The key Potato Bot uses with DeepSeek. Check key sends one tiny message to see that it works.',
      keywords: `${KEYWORDS} api key check test keychain`,
      group: 'Potato Bot',
      render: () => <KeyRow transport={transport} api={api} />,
    },
    {
      id: 'potato-go-key',
      label: 'OpenCode Go key',
      hint: 'The key for your OpenCode Go plan. Pick OpenCode Go as the brain, then choose its model on Models and fallbacks.',
      keywords: `${KEYWORDS} opencode go api key plan models glm kimi deepseek keychain`,
      group: 'Potato Bot',
      render: () => (
        <SettingBlock label="OpenCode Go key" description="Kept in your system keychain. Only the last 4 characters are ever shown here, and the key never goes to the phone, a chat, an export or a log. Without one, the sign-in OpenCode already has on this computer is used.">
          <GoKeyField transport={transport} api={api} />
        </SettingBlock>
      ),
    },
    {
      id: 'potato-persona',
      label: 'Default persona',
      hint: 'The voice a new bot starts with, Fish or Potato.',
      keywords: KEYWORDS,
      group: 'Potato Bot',
      render: (env) => <PersonaRow transport={transport} onSaved={env.markSaved} setError={env.setError} />,
    },
    {
      id: 'potato-limit',
      label: 'Daily limit',
      hint: 'The most one bot may spend in a day. From $0.05 to $20, or No limit.',
      keywords: `${KEYWORDS} dollars money spend no limit unlimited`,
      group: 'Potato Bot',
      render: (env) => (
        <>
          <LimitRow transport={transport} onSaved={env.markSaved} setError={env.setError} />
          <NoLimitRow transport={transport} onSaved={env.markSaved} setError={env.setError} />
        </>
      ),
    },
    connectorsRow,
    mcpRow,
    usageRow,
  ];
}

/** Potato Bot's rows, then the sandbox's (its switch is in General > Labs; its other rows show only while that is on). */
export const potatoRows: RowDef[] = [...makePotatoRows(), ...makePotatoVoiceRows(), ...makeSandboxRows(), ...makeUpdateRows()];
