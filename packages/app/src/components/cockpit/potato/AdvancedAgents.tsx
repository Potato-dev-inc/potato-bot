// "Agents in the sandbox" on the Advanced tab (Model row, "Through my agents"): the owner pastes a definition.toml, reads a plain
// summary (name, command, what it installs, the two notes), and approves it. Then the agent can be picked as this bot's brain.
// Nothing here runs an agent; the engine does, in the sandbox, only for the owner's own messages.
import { useCallback, useEffect, useState } from 'react';
import type { SecretsApi } from '../../../lib/secrets';
import { ApiKeyField } from '../../settings/ApiKeyField';
import { AGENTS_COPY, agentErrorText, tauriAgentsTransport, type AgentSummary, type AgentsTransport, type AgentsView } from '../../../lib/potato-agents';

export function AdvancedAgents({ botId, transport = tauriAgentsTransport, secrets }: { botId: string; transport?: AgentsTransport; secrets?: SecretsApi }) {
  const [view, setView] = useState<AgentsView | null>(null);
  const [adding, setAdding] = useState(false);
  const [text, setText] = useState('');
  const [summary, setSummary] = useState<AgentSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let live = true;
    transport
      .get(botId)
      .then((next) => live && setView(next))
      .catch(() => live && setView(null));
    return () => {
      live = false;
    };
  }, [botId, transport]);

  const run = useCallback(
    async (call: () => Promise<AgentsView>, fallback: string): Promise<boolean> => {
      setBusy(true);
      setError(null);
      try {
        setView(await call());
        return true;
      } catch (cause) {
        setError(agentErrorText(cause, fallback));
        return false;
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  const check = async () => {
    setBusy(true);
    setError(null);
    setSummary(null);
    try {
      setSummary(await transport.check(text));
    } catch (cause) {
      setError(agentErrorText(cause, 'That definition could not be read.'));
    } finally {
      setBusy(false);
    }
  };

  const approve = async () => {
    if (!summary) return;
    const ok = await run(() => transport.add(botId, text, summary.fingerprint), 'Could not add that agent.');
    if (ok) {
      setAdding(false);
      setText('');
      setSummary(null);
    }
  };

  return (
    <div className="ck-potato-adv-agents" role="group" aria-label={AGENTS_COPY.title}>
      <h4 className="ck-potato-adv-line">{AGENTS_COPY.title}</h4>
      <p className="ck-sub">{AGENTS_COPY.line}</p>
      {view && (
        <>
          <p className="ck-sub">{view.decidesNote}</p>
          <p className="ck-sub">{view.ownAccountNote}</p>
          {!view.sandboxReady && (
            <p className="ck-sub" role="status">
              {AGENTS_COPY.noSandbox}
            </p>
          )}
          {view.agents.length === 0 ? (
            <p className="ck-sub">{AGENTS_COPY.none}</p>
          ) : (
            <ul className="ck-potato-adv-agentlist">
              {view.agents.map((agent) => (
                <li key={agent.id}>
                  <strong>{agent.label}</strong> <code>{agent.command}</code>
                  {agent.description && <span className="ck-sub"> {agent.description}</span>}
                  {view.brainAgent === agent.id ? (
                    <span className="ck-sub"> {AGENTS_COPY.inUse}</span>
                  ) : (
                    <button type="button" className="ck-btn ck-btn--secondary" disabled={busy} onClick={() => void run(() => transport.pick(botId, agent.id), 'Could not pick that agent.')}>
                      {AGENTS_COPY.use}
                    </button>
                  )}
                  <button
                    type="button"
                    className="ck-btn ck-btn--plain"
                    disabled={busy}
                    aria-label={`${AGENTS_COPY.remove} ${agent.label}`}
                    onClick={() => void run(() => transport.remove(botId, agent.id), 'Could not remove that agent.')}
                  >
                    {AGENTS_COPY.remove}
                  </button>
                  {agent.secrets.length > 0 && (
                    <div className="ck-potato-adv-agentsecrets" role="group" aria-label={`Values for ${agent.label}`}>
                      <p className="ck-sub">{AGENTS_COPY.secretsLine}</p>
                      {agent.secrets.map((secret) => (
                        <ApiKeyField key={secret.key} label={`${agent.label}: ${secret.env}`} name={secret.key} placeholder={`Value for ${secret.env}`} api={secrets} />
                      ))}
                    </div>
                  )}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
      {!adding ? (
        <button type="button" className="ck-btn ck-btn--secondary" onClick={() => setAdding(true)}>
          {AGENTS_COPY.add}
        </button>
      ) : (
        <div className="ck-potato-adv-agentadd">
          <textarea
            className="ck-potato-adv-input"
            aria-label={AGENTS_COPY.paste}
            placeholder={AGENTS_COPY.paste}
            rows={10}
            value={text}
            onChange={(event) => {
              setText(event.target.value);
              setSummary(null);
            }}
          />
          <button type="button" className="ck-btn ck-btn--secondary" disabled={busy || text.trim() === ''} onClick={() => void check()}>
            {AGENTS_COPY.check}
          </button>
          {summary && (
            <div className="ck-potato-adv-agentsummary" role="region" aria-label="Agent summary">
              <p>
                <strong>{summary.label}</strong> ({summary.id})
              </p>
              {summary.description && <p className="ck-sub">{summary.description}</p>}
              <p>
                Runs: <code>{summary.command}</code>
              </p>
              <p className="ck-sub">{summary.installs}</p>
              {summary.envNames.length > 0 && <p className="ck-sub">Sets: {summary.envNames.join(', ')}</p>}
              {summary.secretNames.length > 0 && (
                <p className="ck-sub">
                  {AGENTS_COPY.asks}: {summary.secretNames.join(', ')}
                </p>
              )}
              {summary.notes.map((note) => (
                <p key={note} className="ck-sub">
                  {note}
                </p>
              ))}
              <button type="button" className="ck-btn" disabled={busy} onClick={() => void approve()}>
                {AGENTS_COPY.approve}
              </button>
            </div>
          )}
        </div>
      )}
      {error && (
        <p className="ck-sub" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
