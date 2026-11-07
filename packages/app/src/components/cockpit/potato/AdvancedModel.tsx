// "Model" on the Advanced tab: which talk brain this bot uses. "Use the default" follows the Settings choice. The
// engine starts the bot on the new brain from its next message. "My TUI agent" also picks the agent and how it runs:
// the companion's own terminal or a Chat (ACP) session it starts in its own folder. A bot never borrows a pane from
// the main work area: its agent and provider belong to the bot. The picker lives here, never in the chat.
import { useEffect, useState } from 'react';
import type { CompanionAgentOption, CompanionTransport, CompanionView } from '../../../lib/companion';
import { chatCapable, tauriAcpChatApi, type AcpLaneAgent } from '../../../lib/acpChat';
import { isTauri } from '../../../lib/cockpit-api';
import { sendPaneMessage } from '../../../lib/paneOutbox';
import type { BrainKind } from '../../../lib/potato';
import { plainError } from '../../../lib/potato';
import { ADVANCED_COPY, brainChoices, brainFromValue, brainValue, type BotProvider } from '../../../lib/potato-botsettings';
import { GO_DEFAULT_MODEL, GO_MODELS, goModelLabel } from '../../../lib/potato-go-models';
import { ApiKeyField } from '../../settings/ApiKeyField';
import {
  SANDBOX_NOTICE,
  acpPaneOf,
  announceAgentChoice,
  readAcpPane,
  readAgentChoice,
  writeAcpPane,
  writeAgentChoice,
} from '../../../lib/potato-agent-choice';
import { AdvancedSection } from './AdvancedSection';

export interface AdvancedModelProps {
  brain: BrainKind | null;
  defaultBrain: BrainKind;
  onChange: (brain: BrainKind | null) => void;
  /** The bot's own provider. `null`: the default in Settings. */
  custom?: BotProvider | null;
  onProviderChange?: (custom: BotProvider | null) => void;
  /** The bot's own model on the Go plan. `null`: the default in Settings (`defaultGoModel`, empty for the brain's own). */
  goModel?: string | null;
  defaultGoModel?: string;
  onGoModelChange?: (model: string | null) => void;
  /** The bot these choices belong to. */
  botId: string;
  /** The bot's companion: its pane and the TUI agent it runs on ("My TUI agent"). */
  companion?: CompanionView | null;
  companionTransport?: CompanionTransport;
  onChanged?: () => void;
  onNotify?: (text: string) => void;
}

export function AdvancedModel({ brain, defaultBrain, onChange, custom, onProviderChange, goModel, defaultGoModel, onGoModelChange, botId, companion, companionTransport, onChanged, onNotify }: AdvancedModelProps) {
  const usingAgents = (brain ?? defaultBrain) === 'agents';
  const usingCustom = (brain ?? defaultBrain) === 'custom';
  const usingGo = (brain ?? defaultBrain) === 'go';
  const [baseUrl, setBaseUrl] = useState(custom?.baseUrl ?? '');
  const [model, setModel] = useState(custom?.model ?? '');
  useEffect(() => {
    setBaseUrl(custom?.baseUrl ?? '');
    setModel(custom?.model ?? '');
  }, [custom?.baseUrl, custom?.model]);
  const saveProvider = (base: string, name: string) => {
    const next = { baseUrl: base.trim(), model: name.trim() };
    onProviderChange?.(next.baseUrl || next.model ? next : null);
  };
  const [agents, setAgents] = useState<CompanionAgentOption[] | null>(null);
  const [acpAgents, setAcpAgents] = useState<AcpLaneAgent[]>([]);
  const [choice, setChoice] = useState<string>(() => readAgentChoice(botId));
  const acpAgentId = acpPaneOf(choice);
  const [acpPane, setAcpPane] = useState<string | null>(() => (acpAgentId ? readAcpPane(botId, acpAgentId) : null));
  const [busy, setBusy] = useState(false);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [told, setTold] = useState(false);
  const tellAboutSandbox = async () => {
    const target = companion?.paneId;
    if (!target) return;
    try {
      await sendPaneMessage(target, SANDBOX_NOTICE);
      setTold(true);
      onNotify?.('Sent the sandbox and Vercel instructions to the agent.');
    } catch (cause) {
      onNotify?.(plainError(cause, 'Could not send that to the agent.'));
    }
  };
  useEffect(() => {
    if (!usingAgents || !companionTransport) return;
    let live = true;
    companionTransport
      .agents()
      .then((list) => live && setAgents(list))
      .catch(() => live && setAgents([]));
    return () => {
      live = false;
    };
  }, [usingAgents, companionTransport]);
  useEffect(() => {
    if (!usingAgents || !isTauri()) return;
    let live = true;
    tauriAcpChatApi
      .laneGet()
      .then((view) => live && setAcpAgents(chatCapable(view)))
      .catch(() => live && setAcpAgents([]));
    return () => {
      live = false;
    };
  }, [usingAgents]);
  const announce = (value: string) => {
    setChoice(value);
    writeAgentChoice(botId, value);
    announceAgentChoice(botId);
  };
  const move = async (agentType: string) => {
    if (!companion || !companionTransport || busy) return;
    setBusy(true);
    try {
      await companionTransport.update(companion.id, { agentType });
      if (!companion.paneId) await companionTransport.wake(companion.id);
      onChanged?.();
    } catch (cause) {
      onNotify?.(plainError(cause, 'Could not change the agent.'));
    } finally {
      setBusy(false);
    }
  };
  const startAcp = async (agentId: string) => {
    if (!companion?.folder) {
      setError('This bot has no home folder to start a chat in.');
      return;
    }
    if (starting) return;
    setStarting(true);
    setError(null);
    try {
      const started = await tauriAcpChatApi.start(agentId, companion.folder, true);
      writeAcpPane(botId, agentId, started.pane_id);
      setAcpPane(started.pane_id);
      announceAgentChoice(botId);
      onChanged?.();
    } catch (cause) {
      setError(plainError(cause, 'Could not start the chat.'));
    } finally {
      setStarting(false);
    }
  };
  const pick = async (value: string) => {
    if (value.startsWith('acp:')) {
      const id = value.slice(4);
      announce(value);
      setAcpPane(readAcpPane(botId, id));
      await startAcp(id);
      return;
    }
    announce('tui');
    if (value.startsWith('tui:') && companion && value.slice(4) !== companion.agentType) {
      await move(value.slice(4));
    }
  };
  const needed = acpAgentId ? acpAgents.find((a) => a.id === acpAgentId)?.label ?? acpAgentId : null;
  const current = acpAgentId ? choice : `tui:${companion?.agentType ?? ''}`;
  return (
    <AdvancedSection title={ADVANCED_COPY.model} line={ADVANCED_COPY.modelLine} id="ck-adv-model">
      <select
        className="ck-potato-adv-input"
        aria-labelledby="ck-adv-model"
        value={brainValue(brain)}
        onChange={(event) => onChange(brainFromValue(event.target.value))}
      >
        {brainChoices(defaultBrain).map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
      {usingGo && (
        <div className="ck-potato-adv-provider">
          <label className="ck-potato-adv-field">
            <span className="ck-potato-adv-line">OpenCode Go model</span>
            <select className="ck-potato-adv-input" aria-label="Go model" value={goModel ?? ''} onChange={(event) => onGoModelChange?.(event.target.value || null)}>
              <option value="">{`Use the default (${goModelLabel(defaultGoModel || GO_DEFAULT_MODEL)})`}</option>
              {goModel && !GO_MODELS.some((m) => m.id === goModel) && <option value={goModel}>{goModel}</option>}
              {GO_MODELS.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.label}
                </option>
              ))}
            </select>
          </label>
          <p className="ck-sub">Only this bot. The Go key is one for all bots, in Settings.</p>
        </div>
      )}
      {usingCustom && (
        <div className="ck-potato-adv-provider">
          <p className="ck-sub">This bot's own provider. Other bots keep theirs.</p>
          <label className="ck-potato-adv-field">
            <span className="ck-potato-adv-line">Base URL</span>
            <input
              className="ck-potato-adv-input"
              aria-label="Provider base URL"
              value={baseUrl}
              placeholder="https://api.openai.com/v1"
              onChange={(event) => setBaseUrl(event.target.value)}
              onBlur={() => saveProvider(baseUrl, model)}
            />
          </label>
          <label className="ck-potato-adv-field">
            <span className="ck-potato-adv-line">Model</span>
            <input
              className="ck-potato-adv-input"
              aria-label="Provider model"
              value={model}
              placeholder="gpt-4.1"
              onChange={(event) => setModel(event.target.value)}
              onBlur={() => saveProvider(baseUrl, model)}
            />
          </label>
          <ApiKeyField
            label="Provider key"
            name={`potato_provider_key_${botId}`}
            placeholder="The key for this bot's provider"
            onSaved={() => onProviderChange?.(baseUrl.trim() || model.trim() ? { baseUrl: baseUrl.trim(), model: model.trim() } : null)}
          />
        </div>
      )}
      {usingAgents && (
        <div className="ck-potato-adv-agent">
          <label className="ck-sub" htmlFor="ck-adv-agent">
            Agent and how it runs
          </label>
          {companion ? (
            <select id="ck-adv-agent" className="ck-potato-adv-input" value={current} disabled={busy || starting} onChange={(event) => void pick(event.target.value)}>
              <optgroup label="Terminal (TUI)">
                {agents === null ? (
                  <option value={`tui:${companion.agentType}`}>{companion.agentLabel} (looking for agents...)</option>
                ) : (
                  <>
                    {!agents.some((a) => a.id === companion.agentType) && <option value={`tui:${companion.agentType}`}>{companion.agentLabel}</option>}
                    {agents.map((agent) => (
                      <option key={agent.id} value={`tui:${agent.id}`} disabled={!agent.ok}>
                        {agent.label}
                        {agent.ok ? '' : ` (${agent.reason ?? 'not available'})`}
                      </option>
                    ))}
                  </>
                )}
              </optgroup>
              {acpAgents.length > 0 && (
                <optgroup label="Chat (ACP)">
                  {acpAgents.map((agent) => (
                    <option key={`acp-${agent.id}`} value={`acp:${agent.id}`} disabled={agent.disabled}>
                      {agent.label}
                      {agent.disabled && agent.disabled_note ? ` (${agent.disabled_note})` : ''}
                    </option>
                  ))}
                </optgroup>
              )}
            </select>
          ) : (
            <p className="ck-sub">Make the companion first, then pick the agent it runs on.</p>
          )}
          <p className="ck-sub">
            Terminal keeps the agent's own screen. Chat (ACP) starts a chat session in the bot's own folder, with the agent's model, mode and effort chips.
          </p>
          {acpAgentId && !acpPane && (
            <div className="ck-potato-adv-agentstart">
              <p className="ck-sub" role={error ? 'alert' : undefined}>
                {error ?? `Start ${needed} as a chat session.`}
              </p>
              <button type="button" className="ck-btn ck-btn--secondary" disabled={starting} onClick={() => void startAcp(acpAgentId)}>
                {starting ? 'Starting…' : 'Start chat'}
              </button>
            </div>
          )}
          {acpPane && <p className="ck-sub">The chat is running in pane {acpPane}.</p>}
          {companion?.paneId && (
            <div className="ck-potato-adv-agentstart">
              <button type="button" className="ck-btn ck-btn--plain" onClick={() => void tellAboutSandbox()}>
                {told ? 'Instructions sent' : 'Tell the agent about the computer and Vercel'}
              </button>
              <p className="ck-sub">Sends the exact sandbox and Vercel commands to the agent now, without restarting it.</p>
            </div>
          )}
        </div>
      )}
    </AdvancedSection>
  );
}

