// Settings > Potato Bot > Connectors: outside services the bot can use (GitHub, Notion). A short list the app ships, each
// from the maker's own server. The token is typed here, in this field, once: it goes to the PC's credential store and to
// the program on the bot's computer, and never comes back to this page, the chat or the bot. Below the list, "Add your
// own" is the MCP servers form for anything the list does not have.
import { useEffect, useState } from 'react';
import { plainError } from '../../../lib/potato';
import { MCP_NAME_PATTERN, tauriMcpTransport, type CatalogEntry, type McpNewLink, type McpOverview, type McpServerView, type McpTestResult, type McpTransport } from '../../../lib/potato-mcp';
import { tauri } from '../../../lib/tauri';
import { Switch } from '../controls';
import { Btn, FieldInput, Note, SettingBlock } from '../settings-ui';

export const CONNECTORS_COPY = {
  heading: 'Connectors',
  intro: 'Let the bot use a service you already have. You connect each one yourself, here. The bot can ask for one, but it can never ask you for a token in chat.',
  privacy: "The token stays on this PC. It is kept in the credential store and handed only to this service's program on the bot's computer. The bot, the chat and the logs never see it.",
  reads: 'Always allow reads',
  readsHint: 'Reads from this service run without asking. Changes still follow When to ask.',
  readsOff: 'Without this, each read asks you first, because it reads your real account.',
  text: 'What a service sends back is treated as data, not as orders.',
  custom: 'Not on the list? Add your own below.',
  toolsHelp: 'Untick a tool to stop this bot using it. The app refuses it every time, whatever the bot says.',
  toolsLimited: 'Some tools are switched off for this bot.',
  signInHelp: "Sign in opens the service's own page in your browser. The sign-in stays on this PC; the bot never sees it.",
  signInWaiting: 'Finish the sign-in in your browser. This waits up to 5 minutes.',
  signedIn: 'Signed in',
  clientIdHint: 'Client id (only if the service asks for one)',
  linkHeading: 'Add by link',
  linkHelp: 'A tool server on the internet, by its https link. This app talks to it from this PC; the bot never sees the token.',
  linkLocal: 'Local',
  linkLocalHint: 'Tick only for a server on this PC or your own network. Plain http is allowed there and nowhere else.',
  linkPrivacy: "A token is optional. It stays on this PC and is sent only to this link's own address.",
  linkNameError: 'Use 1 to 32 characters for the name: lowercase letters, numbers, - and _.',
};

function Connected({
  server,
  busy,
  test,
  onToggle,
  onReads,
  onTest,
  onRemove,
  onTools,
  onSignIn,
  onSignOut,
}: {
  server: McpServerView;
  busy: boolean;
  test: McpTestResult | undefined;
  onToggle: (on: boolean) => void;
  onReads: (on: boolean) => void;
  onTest: () => void;
  onRemove: () => void;
  onTools: (allow: string[]) => void;
  /** By link only. */
  onSignIn?: (clientId: string | null) => void;
  onSignOut?: () => void;
}) {
  const [clientId, setClientId] = useState('');
  return (
    <>
      <div className="pot-mcp-actions">
        <Switch checked={server.enabled} onChange={onToggle} label={`Use ${server.name} for this bot`} disabled={busy} />
        <Btn small aria-label={`Test ${server.name}`} disabled={busy} onClick={onTest}>
          Test
        </Btn>
        <Btn small aria-label={`Disconnect ${server.name}`} disabled={busy} onClick={onRemove}>
          Disconnect
        </Btn>
      </div>
      {onSignIn && onSignOut && (
        <>
          <div className="pot-mcp-actions">
            {server.signedIn ? (
              <>
                <span className="st-value">{CONNECTORS_COPY.signedIn}</span>
                <Btn small aria-label={`Sign out of ${server.name}`} disabled={busy} onClick={onSignOut}>
                  Sign out
                </Btn>
              </>
            ) : (
              <Btn small aria-label={`Sign in to ${server.name}`} disabled={busy} onClick={() => onSignIn(clientId.trim() === '' ? null : clientId.trim())}>
                Sign in
              </Btn>
            )}
          </div>
          {!server.signedIn && (
            <FieldInput aria-label={`Client id for ${server.name}`} placeholder={CONNECTORS_COPY.clientIdHint} spellCheck={false} value={clientId} onChange={(e) => setClientId(e.target.value)} />
          )}
          <Note>{CONNECTORS_COPY.signInHelp}</Note>
        </>
      )}
      <div className="pot-mcp-actions">
        <Switch checked={server.alwaysAllowReads} onChange={onReads} label={`${CONNECTORS_COPY.reads} for ${server.name}`} disabled={busy || !server.enabled} />
        <span className="st-value">{CONNECTORS_COPY.reads}</span>
      </div>
      <Note>{server.alwaysAllowReads ? CONNECTORS_COPY.readsHint : CONNECTORS_COPY.readsOff}</Note>
      {test && <Note role={test.ok ? 'status' : 'alert'}>{test.ok ? (test.tools.length ? `Tools: ${test.tools.join(', ')}` : 'Works, no tools listed.') : (test.error ?? 'The test failed.')}</Note>}
      {test?.ok && test.tools.length > 0 && <ToolList server={server} tools={test.tools} busy={busy} onSave={onTools} />}
      {!(test?.ok && test.tools.length > 0) && (server.allowTools.length > 0 || server.blockTools.length > 0) && (
        <Note>{`${CONNECTORS_COPY.toolsLimited} Press Test to change which.`}</Note>
      )}
    </>
  );
}

/** Which of a server's tools the bot may call. Unticked tools are refused by the app on every call, so the bot cannot use them
 *  even by asking for them another way. Saved as the allow list; the app also keeps a block list that this page leaves as it is. */
function ToolList({ server, tools, busy, onSave }: { server: McpServerView; tools: string[]; busy: boolean; onSave: (allow: string[]) => void }) {
  const initial = (t: string) => !server.blockTools.includes(t) && (server.allowTools.length === 0 || server.allowTools.includes(t));
  const [on, setOn] = useState<Record<string, boolean>>(() => Object.fromEntries(tools.map((t) => [t, initial(t)])));
  const chosen = tools.filter((t) => on[t]);
  return (
    <>
      <Note>{CONNECTORS_COPY.toolsHelp}</Note>
      {tools.map((t) => (
        <div key={t} className="pot-mcp-actions">
          <Switch checked={on[t] ?? true} onChange={(v) => setOn((o) => ({ ...o, [t]: v }))} label={`Allow ${t} on ${server.name}`} disabled={busy} />
          <span className="st-value">{t}</span>
        </div>
      ))}
      <Btn small aria-label={`Save tools for ${server.name}`} disabled={busy} onClick={() => onSave(chosen.length === tools.length ? [] : chosen)}>
        Save tools
      </Btn>
    </>
  );
}

function ConnectForm({ entry, busy, onConnect }: { entry: CatalogEntry; busy: boolean; onConnect: (values: Record<string, string>) => Promise<boolean> }) {
  const [values, setValues] = useState<Record<string, string>>({});
  const ready = entry.fields.every((f) => (values[f.key] ?? '').trim() !== '');
  return (
    <>
      {entry.fields.map((f) => (
        <div key={f.key}>
          <FieldInput
            type="password"
            autoComplete="off"
            spellCheck={false}
            aria-label={`${entry.title} ${f.label}`}
            placeholder={f.label}
            value={values[f.key] ?? ''}
            onChange={(e) => setValues((v) => ({ ...v, [f.key]: e.target.value }))}
          />
          {f.help && <Note>{f.help}</Note>}
        </div>
      ))}
      <Note>{CONNECTORS_COPY.privacy}</Note>
      <Btn
        primary
        aria-label={`Connect ${entry.title}`}
        disabled={busy || !ready}
        onClick={() =>
          void onConnect(values).then((ok) => {
            // The typed text leaves this page's memory as soon as the connect worked.
            if (ok) setValues({});
          })
        }
      >
        Connect
      </Btn>
    </>
  );
}

function LinkForm({ busy, onAdd }: { busy: boolean; onAdd: (link: McpNewLink) => Promise<boolean> }) {
  const [name, setName] = useState('');
  const [url, setUrl] = useState('');
  const [token, setToken] = useState('');
  const [local, setLocal] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const submit = async () => {
    setProblem(null);
    if (!MCP_NAME_PATTERN.test(name)) return setProblem(CONNECTORS_COPY.linkNameError);
    if (!url.trim()) return setProblem('Paste the https link of the tool server.');
    const ok = await onAdd({ name, url: url.trim(), local, token: token.trim() === '' ? null : token.trim() });
    if (ok) {
      // The typed token leaves this page's memory as soon as the add worked.
      setName('');
      setUrl('');
      setToken('');
      setLocal(false);
    }
  };
  return (
    <SettingBlock label={CONNECTORS_COPY.linkHeading} description={CONNECTORS_COPY.linkHelp}>
      <FieldInput aria-label="Link name" placeholder="name" value={name} onChange={(e) => setName(e.target.value)} />
      <FieldInput aria-label="Tool server link" placeholder="https://example.com/mcp" spellCheck={false} value={url} onChange={(e) => setUrl(e.target.value)} />
      <FieldInput type="password" autoComplete="off" spellCheck={false} aria-label="Link token" placeholder="Token (optional)" value={token} onChange={(e) => setToken(e.target.value)} />
      <div className="pot-mcp-actions">
        <Switch checked={local} onChange={setLocal} label={CONNECTORS_COPY.linkLocal} disabled={busy} />
        <span className="st-value">{CONNECTORS_COPY.linkLocal}</span>
      </div>
      <Note>{CONNECTORS_COPY.linkLocalHint}</Note>
      <Note>{CONNECTORS_COPY.linkPrivacy}</Note>
      {problem && <Note role="alert">{problem}</Note>}
      <Btn primary aria-label="Add link" disabled={busy} onClick={() => void submit()}>
        Add
      </Btn>
    </SettingBlock>
  );
}

export function ConnectorsSection({ botId, transport = tauriMcpTransport }: { botId: string; transport?: McpTransport }) {
  const [overview, setOverview] = useState<McpOverview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [tests, setTests] = useState<Record<string, McpTestResult>>({});
  const [signing, setSigning] = useState(false);

  useEffect(() => {
    let live = true;
    transport.overview(botId).then(
      (o) => live && setOverview(o),
      (e: unknown) => live && setError(plainError(e, 'Could not load connectors.')),
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

  const runTest = async (name: string) => {
    setBusy(true);
    setError(null);
    try {
      const result = await transport.test(botId, name);
      setTests((t) => ({ ...t, [name]: result }));
    } catch (e) {
      setError(plainError(e, 'Could not test that connector.'));
    } finally {
      setBusy(false);
    }
  };

  const connectors = overview?.catalog.filter((c) => c.connector) ?? [];
  const links = overview?.servers.filter((s) => s.url !== null) ?? [];
  return (
    <div className="pot-mcp">
      <h3>{CONNECTORS_COPY.heading}</h3>
      <p>{CONNECTORS_COPY.intro}</p>
      {error && <Note role="alert">{error}</Note>}
      {busy && signing && <Note role="status">{CONNECTORS_COPY.signInWaiting}</Note>}
      {!overview && !error && <Note>Loading…</Note>}
      {overview && (
        <>
          {!overview.builtin.ready && <Note>{`The bot's computer is not running. Start it to connect a service. ${overview.builtin.reason ?? ''}`.trim()}</Note>}
          {connectors.map((c) => {
            const server = overview.servers.find((s) => s.name === c.name && s.connector);
            const about = [c.description, c.maker ? `Made by: ${c.maker}.` : null].filter(Boolean).join(' ');
            return (
              <SettingBlock key={c.id} label={c.title} description={server ? [about, server.note].filter(Boolean).join(' ') : about}>
                {c.docs && (
                  <Btn small aria-label={`About ${c.title}`} onClick={() => void Promise.resolve(tauri.openExternalUrl(c.docs!)).catch(() => {})}>
                    About
                  </Btn>
                )}
                {server ? (
                  <Connected
                    server={server}
                    busy={busy}
                    test={tests[c.name]}
                    onToggle={(on) => void run(() => transport.setEnabled(botId, c.name, on), 'Could not change that connector.')}
                    onReads={(on) => void run(() => transport.setAlwaysAllowReads(botId, c.name, on), 'Could not change that.')}
                    onTest={() => void runTest(c.name)}
                    onRemove={() => void run(() => transport.remove(botId, c.name), 'Could not disconnect that connector.')}
                    onTools={(allow) => void run(() => transport.setTools(botId, c.name, allow, server.blockTools), 'Could not save the tools.')}
                  />
                ) : (
                  <ConnectForm entry={c} busy={busy || !overview.builtin.ready} onConnect={(values) => run(() => transport.connect(botId, c.id, values), 'Could not connect that service.')} />
                )}
              </SettingBlock>
            );
          })}
          {links.map((server) => (
            <SettingBlock key={server.name} label={server.name} description={[server.url, server.local ? 'Local' : null, server.note].filter(Boolean).join(' · ')}>
              <Connected
                server={server}
                busy={busy}
                test={tests[server.name]}
                onToggle={(on) => void run(() => transport.setEnabled(botId, server.name, on), 'Could not change that connector.')}
                onReads={(on) => void run(() => transport.setAlwaysAllowReads(botId, server.name, on), 'Could not change that.')}
                onTest={() => void runTest(server.name)}
                onRemove={() => void run(() => transport.remove(botId, server.name), 'Could not disconnect that connector.')}
                onTools={(allow) => void run(() => transport.setTools(botId, server.name, allow, server.blockTools), 'Could not save the tools.')}
                onSignIn={(clientId) => {
                  setSigning(true);
                  void run(() => transport.signIn(botId, server.name, clientId), 'Could not sign in.').finally(() => setSigning(false));
                }}
                onSignOut={() => void run(() => transport.signOut(botId, server.name), 'Could not sign out.')}
              />
            </SettingBlock>
          ))}
          <LinkForm busy={busy} onAdd={(link) => run(() => transport.addLink(botId, link), 'Could not add that link.')} />
          <Note>{CONNECTORS_COPY.text}</Note>
          <Note>{CONNECTORS_COPY.custom}</Note>
        </>
      )}
    </div>
  );
}
