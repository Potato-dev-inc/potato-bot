// Settings > Potato Bot > Models and fallbacks (a sub-page). The talk brain choice (moved here from the Potato Bot page,
// the same control) and the ordered fallback list: when the talk brain is out of balance or not answering, the next
// brain in the list is tried for that message. Keys are not here: each brain keeps its own (the keychain, or its sign-in).
import { useState } from 'react';
import { brainLabel, plainError, type BrainKind, type PotatoSettings, type PotatoTransport } from '../../../lib/potato';
import { GO_DEFAULT_MODEL, GO_MODELS } from '../../../lib/potato-go-models';
import { ApiKeyField } from '../ApiKeyField';
import { GoKeyField } from './GoKeyField';
import { Btn, FieldInput, FieldSelect, Note, SettingBlock, SettingRow } from '../settings-ui';
import { usePotatoSettings } from './usePotatoSettings';

/** What the talk brain does and costs, under its name. */
export const BRAIN_NOTE: Record<BrainKind, string> = {
  deepseek: 'Pays DeepSeek per message. Usually cents a day.',
  go: 'Uses your OpenCode Go key, or the sign-in OpenCode already has on this computer.',
  agents: 'Runs on one of your agents on this machine: Claude, Codex, Cursor, OpenCode and so on. Pick it on the bot or on the companion.',
  custom: 'Any OpenAI-compatible API endpoint: OpenAI, OpenRouter, a local server, or another provider. You pay that provider.',
};
const BRAINS: BrainKind[] = ['deepseek', 'go', 'custom', 'agents'];
/** The brains that can be a fallback: the ones that run a model. "Through my agents" runs no brain of its own. */
const FALLBACK_BRAINS: BrainKind[] = ['deepseek', 'go'];

/** The list with the brain at `from` moved to `to` (the other brains keep their order). Out of range: the same list. */
export function moveBrain(list: BrainKind[], from: number, to: number): BrainKind[] {
  if (from < 0 || to < 0 || from >= list.length || to >= list.length || from === to) return [...list];
  const next = [...list];
  const [kind] = next.splice(from, 1);
  next.splice(to, 0, kind);
  return next;
}

type Message = { ok: boolean; text: string } | null;

export function ModelsPage({ transport }: { transport: PotatoTransport }) {
  const { settings, failed, save } = usePotatoSettings(transport);
  const [message, setMessage] = useState<Message>(null);
  const [pick, setPick] = useState<BrainKind | ''>('');

  if (!settings) {
    return failed ? <Note role="alert">{failed}</Note> : <span className="pot-sub-loading" aria-busy="true">Loading…</span>;
  }

  const brain = settings.brain;
  const fallbacks = settings.fallbacks ?? [];
  const off = brain === 'agents';
  const addable = FALLBACK_BRAINS.filter((kind) => kind !== brain && !fallbacks.includes(kind));

  const write = async (change: Partial<PotatoSettings>, done: string) => {
    setMessage(null);
    try {
      await save(change);
      setMessage({ ok: true, text: done });
    } catch (error) {
      setMessage({ ok: false, text: plainError(error, 'Could not save that.') });
    }
  };

  const move = (index: number, to: number) => void write({ fallbacks: moveBrain(fallbacks, index, to) }, 'Order saved.');

  return (
    <div className="pot-models">
      <SettingRow label="Talk brain" description={BRAIN_NOTE[brain]}>
        <FieldSelect
          aria-label="Talk brain"
          value={brain}
          onChange={(e) => void write({ brain: e.target.value as BrainKind }, 'Saved.')}
        >
          {BRAINS.map((kind) => (
            <option key={kind} value={kind}>
              {brainLabel(kind)}
            </option>
          ))}
        </FieldSelect>
      </SettingRow>

      {brain === 'custom' && (
        <SettingBlock
          label="My provider"
          description="An OpenAI-compatible endpoint (ending in /v1 is usual). The key is kept in your system keychain and only the bot's brain sees it."
        >
          <SettingRow label="Base URL">
            <FieldInput
              aria-label="Provider base URL"
              defaultValue={settings.custom?.baseUrl ?? ''}
              placeholder="https://api.openai.com/v1"
              onBlur={(e) => void write({ custom: { baseUrl: e.target.value.trim(), model: settings.custom?.model ?? '' } }, 'Saved.')}
            />
          </SettingRow>
          <SettingRow label="Model">
            <FieldInput
              aria-label="Provider model"
              defaultValue={settings.custom?.model ?? ''}
              placeholder="gpt-4.1"
              onBlur={(e) => void write({ custom: { baseUrl: settings.custom?.baseUrl ?? '', model: e.target.value.trim() } }, 'Saved.')}
            />
          </SettingRow>
          <ApiKeyField label="Provider key" name="potato_provider_key" placeholder="The key for that provider" onSaved={() => void write({}, 'Saved.')} />
        </SettingBlock>
      )}

      {brain === 'go' && (
        <SettingBlock label="OpenCode Go" description="Your Go plan key is kept in your system keychain and only the bot's brain sees it. A bot can pick its own model on its Advanced tab.">
          <SettingRow label="Model">
            <FieldSelect aria-label="Go model" value={settings.goModel ?? ''} onChange={(e) => void write({ goModel: e.target.value }, 'Saved.')}>
              <option value="">{`Default (${GO_DEFAULT_MODEL})`}</option>
              {settings.goModel && !GO_MODELS.some((m) => m.id === settings.goModel) && <option value={settings.goModel}>{settings.goModel}</option>}
              {GO_MODELS.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.label}
                </option>
              ))}
            </FieldSelect>
          </SettingRow>
          <GoKeyField transport={transport} />
        </SettingBlock>
      )}

      <SettingBlock
        label="Fallbacks"
        description="If the talk brain is busy, out of balance or not answering, the next brain in this list is tried for that message, in order. Each new run starts on the talk brain again. Each brain keeps its own key or sign-in."
      >
        {off && <Note>Fallbacks do not apply to Through my agents.</Note>}
        {fallbacks.length === 0 ? (
          <span className="pot-sub-empty">None yet. If the talk brain fails, you will see the error.</span>
        ) : (
          <ol className="pot-fb-list" aria-label="Fallback brains, in order">
            {fallbacks.map((kind, index) => (
              <li key={kind} className="pot-fb-item">
                <span className="pot-fb-name">
                  <span className="pot-fb-index">{index + 1}.</span> {brainLabel(kind)}
                </span>
                <span className="pot-fb-actions">
                  <Btn small disabled={off || index === 0} aria-label={`Move ${brainLabel(kind)} up`} onClick={() => move(index, index - 1)}>
                    Up
                  </Btn>
                  <Btn small disabled={off || index === fallbacks.length - 1} aria-label={`Move ${brainLabel(kind)} down`} onClick={() => move(index, index + 1)}>
                    Down
                  </Btn>
                  <Btn
                    small
                    disabled={off}
                    aria-label={`Remove ${brainLabel(kind)}`}
                    onClick={() => void write({ fallbacks: fallbacks.filter((k) => k !== kind) }, 'Removed.')}
                  >
                    Remove
                  </Btn>
                </span>
              </li>
            ))}
          </ol>
        )}
        {addable.length > 0 && !off && (
          <div className="pot-fb-add">
            <FieldSelect aria-label="Add a fallback" value={pick} onChange={(e) => setPick(e.target.value as BrainKind | '')}>
              <option value="">Choose a brain</option>
              {addable.map((kind) => (
                <option key={kind} value={kind}>
                  {brainLabel(kind)}
                </option>
              ))}
            </FieldSelect>
            <Btn
              disabled={pick === ''}
              onClick={() => {
                if (pick === '') return;
                setPick('');
                void write({ fallbacks: [...fallbacks, pick] }, 'Added.');
              }}
            >
              Add
            </Btn>
          </div>
        )}
        {!off && addable.length === 0 && fallbacks.length > 0 && <span className="pot-sub-empty">Every other brain is in the list.</span>}
        {message && <Note role={message.ok ? 'status' : 'alert'}>{message.text}</Note>}
      </SettingBlock>
    </div>
  );
}
