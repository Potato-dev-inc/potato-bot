// Settings > Potato Bot > MCP servers: the programs a bot may use on its computer. Reads and changes go through the
// `potato_mcp_*` commands (src/lib/potato-mcp.ts). Env values are typed here once and dropped as soon as the add works;
// the backend never sends them back, only their names.
import { useEffect, useState } from 'react';
import { plainError } from '../../../lib/potato';
import { MCP_NAME_PATTERN, tauriMcpTransport, type McpOverview, type McpServerView, type McpTestResult, type McpTransport } from '../../../lib/potato-mcp';
import { Switch } from '../controls';
import { Btn, FieldInput, Note, SettingBlock, SettingRow } from '../settings-ui';

export const MCP_COPY = {
  heading: 'MCP servers',
  intro: 'These tools let the bot use programs on its computer.',
  warning: "Custom servers run inside the bot's computer and can reach what that computer's walls allow.",
  nameError: 'Use 1 to 32 characters: lowercase letters, numbers, - and _.',
  status: { off: 'Off', ready: 'Ready', starting: 'Starting', failed: 'Failed', unknown: 'Not checked' } as Record<string, string>,
};

/** Arguments: one per line; blank lines are dropped. */
export function parseArgs(text: string): string[] {
  return text.split('\n').map((l) => l.trim()).filter(Boolean);
}

/** Env: KEY=value lines. Returns null for a line that is not KEY=value. */
export function parseEnv(text: string): Record<string, string> | null {
  const out: Record<string, string> = {};
  for (const raw of text.split('\n')) {
    const line = raw.trim();
    if (!line) continue;
    const eq = line.indexOf('=');
    if (eq < 1) return null;
    out[line.slice(0, eq).trim()] = line.slice(eq + 1);
  }
  return out;
}

function ServerRow({ server, busy, test, onToggle, onTest, onRemove }: { server: McpServerView; busy: boolean; test: McpTestResult | undefined; onToggle: (on: boolean) => void; onTest: () => void; onRemove: () => void }) {
  const bits = [MCP_COPY.status[server.status] ?? server.status];
  if (server.toolCount !== null) bits.push(`${server.toolCount} tools`);
  const description = [bits.join(' · '), server.note, server.envKeys.length ? `Keys: ${server.envKeys.join(', ')}` : null].filter(Boolean).join('. ');
  return (
    <SettingBlock label={server.name} description={description}>
      <div className="pot-mcp-actions">
        <Switch checked={server.enabled} onChange={onToggle} label={`Enable ${server.name}`} disabled={busy} />
        <Btn small aria-label={`Test ${server.name}`} disabled={busy} onClick={onTest}>
          Test
        </Btn>
        <Btn small aria-label={`Remove ${server.name}`} disabled={busy} onClick={onRemove}>
          Remove
        </Btn>
      </div>
      {test && <Note role={test.ok ? 'status' : 'alert'}>{test.ok ? (test.tools.length ? `Tools: ${test.tools.join(', ')}` : 'Works, no tools listed.') : (test.error ?? 'The test failed.')}</Note>}
    </SettingBlock>
  );
}

export function McpServersSection({ botId, transport = tauriMcpTransport }: { botId: string; transport?: McpTransport }) {
  const [overview, setOverview] = useState<McpOverview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [tests, setTests] = useState<Record<string, McpTestResult>>({});
  const [name, setName] = useState('');
  const [command, setCommand] = useState('');
  const [args, setArgs] = useState('');
  const [env, setEnv] = useState('');
  const [formError, setFormError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    transport.overview(botId).then(
      (o) => live && setOverview(o),
      (e: unknown) => live && setError(plainError(e, 'Could not load MCP servers.')),
    );
    return () => {
      live = false;
    };
  }, [transport, botId]);

  const run = async (job: () => Promise<McpOverview>, fallback: string): Promise<boolean> => {
    setBusy(true);
    setError(null);
    try {
      setOverview(await job());
      return true;
    } catch (e) {
      setError(plainError(e, fallback));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const submit = async () => {
    setFormError(null);
    if (!MCP_NAME_PATTERN.test(name)) return setFormError(MCP_COPY.nameError);
    if (!command.trim()) return setFormError('Enter the command to run.');
    const parsed = parseEnv(env);
    if (!parsed) return setFormError('Write each key as KEY=value, one per line.');
    const ok = await run(() => transport.add(botId, { name, command: command.trim(), args: parseArgs(args), env: parsed }), 'Could not add that server.');
    if (ok) {
      setName('');
      setCommand('');
      setArgs('');
      setEnv('');
    }
  };

  const runTest = async (server: string) => {
    setBusy(true);
    setError(null);
    try {
      const result = await transport.test(botId, server);
      setTests((t) => ({ ...t, [server]: result }));
    } catch (e) {
      setError(plainError(e, 'Could not test that server.'));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="pot-mcp">
      <h3>{MCP_COPY.heading}</h3>
      <p>{MCP_COPY.intro}</p>
      {error && <Note role="alert">{error}</Note>}
      {!overview && !error && <Note>Loading…</Note>}
      {overview && (
        <>
          <SettingRow label="Computer" description={overview.builtin.ready ? 'Ready' : (overview.builtin.reason ?? 'Not ready')} />
          {overview.catalog.filter((c) => !c.connector).map((c) => (
            <SettingRow key={c.id} label={c.title} description={c.description}>
              {c.installed ? (
                <span className="st-value">Added</span>
              ) : (
                <Btn aria-label={`Add ${c.title}`} disabled={busy} onClick={() => void run(() => transport.catalogAdd(botId, c.id), 'Could not add that server.')}>
                  Add
                </Btn>
              )}
            </SettingRow>
          ))}
          <SettingBlock label="Add your own" description="Arguments: one per line. Keys: KEY=value, one per line.">
            <FieldInput aria-label="Server name" placeholder="name" value={name} onChange={(e) => setName(e.target.value)} />
            <FieldInput aria-label="Server command" placeholder="command" value={command} onChange={(e) => setCommand(e.target.value)} />
            <textarea className="st-field is-mono" aria-label="Server arguments" placeholder="arguments, one per line" rows={3} value={args} onChange={(e) => setArgs(e.target.value)} />
            <textarea className="st-field is-mono" aria-label="Server keys" placeholder="KEY=value, one per line" rows={3} value={env} onChange={(e) => setEnv(e.target.value)} />
            <Note>{MCP_COPY.warning}</Note>
            {formError && <Note role="alert">{formError}</Note>}
            <Btn primary disabled={busy} onClick={() => void submit()}>
              Add server
            </Btn>
          </SettingBlock>
          {overview.servers.filter((s) => !s.connector).map((s) => (
            <ServerRow
              key={s.name}
              server={s}
              busy={busy}
              test={tests[s.name]}
              onToggle={(on) => void run(() => transport.setEnabled(botId, s.name, on), 'Could not change that server.')}
              onTest={() => void runTest(s.name)}
              onRemove={() => void run(() => transport.remove(botId, s.name), 'Could not remove that server.')}
            />
          ))}
        </>
      )}
    </div>
  );
}
