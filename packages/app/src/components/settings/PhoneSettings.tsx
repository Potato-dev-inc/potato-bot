// Settings > Phone: use your bots from your phone. Pair with a QR, see and sign out devices, and the "can't connect"
// help (Windows Firewall, a tunnel from outside your Wi-Fi, your own address, the port). The one-time pairing link
// lives only in this component's state and is dropped when it closes. Nothing here ever sees or logs a phone token.
import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { makePhoneApi, PHONE_CHANGED_EVENT, phoneErrorText, type PairVia, type PhoneApi, type PhoneDevice, type PhoneStatus } from '../../lib/phone-api';
import { ConfirmSheet, Switch } from './controls';
import { availableWays, countdownText, defaultWay, groupCode, lastUsedText, pairedText, qrDataUrl } from './phone-model';
import { Btn, FieldInput, FieldLabel, FieldSelect, Note, SettingBlock } from './settings-ui';

export const TUNNEL_SENTENCE =
  "This makes your PC reachable from the internet through a Cloudflare tunnel. Only phones you paired can use it, and every request needs the phone's token. The address changes if the tunnel restarts, and then you scan a new QR. Cloudflare says this free tunnel is for testing and may stop at any time.";
export const POLL_MS = 1000;
export const POLL_MAX_MS = 40_000;

const tauriApi: PhoneApi = makePhoneApi((command, args) => invoke(command, args));
const tauriListen = (cb: () => void): Promise<() => void> => listen(PHONE_CHANGED_EVENT, cb);

interface Pairing {
  code: string;
  expiresAtMs: number;
  qr: string;
  via: PairVia;
  /** The tunnel address this QR was made for (only when it is a tunnel QR). */
  tunnelUrl: string | null;
}

interface Ask {
  title: string;
  body: string;
  confirmLabel: string;
  run: () => void;
}

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
const asVia = (via: string): PairVia => (via === 'custom' || via === 'tunnel' ? via : 'lan');

const TUNNEL_PHASE_TEXT: Record<string, string> = {
  off: 'Off',
  starting: 'Starting…',
  running: 'On',
  reconnecting: 'Reconnecting…',
  error: 'Something went wrong',
};

export function PhoneSettings({ api = tauriApi, subscribe = tauriListen }: { api?: PhoneApi; subscribe?: (cb: () => void) => Promise<() => void> }) {
  const [status, setStatus] = useState<PhoneStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pairing, setPairing] = useState<Pairing | null>(null);
  const [wayPick, setWayPick] = useState<PairVia | null>(null);
  const [ask, setAsk] = useState<Ask | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const [portText, setPortText] = useState('');
  const [portMsg, setPortMsg] = useState<string | null>(null);
  const [addrText, setAddrText] = useState('');
  const [addrMsg, setAddrMsg] = useState<string | null>(null);
  const alive = useRef(true);
  const pairingRef = useRef<Pairing | null>(null);
  const statusRef = useRef<PhoneStatus | null>(null);
  pairingRef.current = pairing;
  statusRef.current = status;

  const apply = useCallback((next: PhoneStatus) => {
    if (!alive.current) return;
    statusRef.current = next;
    setStatus(next);
  }, []);

  const refresh = useCallback(async () => {
    try {
      apply(await api.status());
    } catch (e) {
      if (alive.current) setError(phoneErrorText(e, 'Could not read the phone settings.'));
    }
  }, [api, apply]);

  useEffect(() => {
    alive.current = true;
    void refresh();
    let off: (() => void) | null = null;
    let closed = false;
    void subscribe(() => void refresh()).then(
      (fn) => {
        if (closed) fn();
        else off = fn;
      },
      () => undefined,
    );
    return () => {
      alive.current = false;
      closed = true;
      off?.();
      pairingRef.current = null;
    };
  }, [refresh, subscribe]);

  // Fill the advanced fields from the saved values (and again when they change elsewhere).
  const savedPort = status?.port;
  const savedAddr = status?.customAddress ?? '';
  useEffect(() => {
    if (savedPort !== undefined) setPortText(String(savedPort));
  }, [savedPort]);
  useEffect(() => {
    setAddrText(savedAddr);
  }, [savedAddr]);

  // The clock ticks only while a QR is shown or a device list needs "last used" to stay fresh.
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);

  const startPairing = useCallback(
    async (opts: { via?: PairVia; relinkDeviceId?: string }) => {
      setError(null);
      try {
        const started = await api.pairingStart(opts);
        if (!alive.current) return;
        const via = asVia(started.via);
        setNow(Date.now());
        setPairing({
          code: started.code,
          expiresAtMs: started.expiresAtSecs * 1000,
          qr: qrDataUrl(started.qrUrl),
          via,
          tunnelUrl: via === 'tunnel' ? statusRef.current?.tunnel.url ?? null : null,
        });
      } catch (e) {
        if (alive.current) setError(phoneErrorText(e, 'Could not make a pairing code.'));
      }
    },
    [api],
  );

  // The tunnel address changed while a tunnel QR is up: tell the person and make a fresh one.
  const tunnelUrl = status?.tunnel.url ?? null;
  useEffect(() => {
    const p = pairingRef.current;
    if (!p || p.via !== 'tunnel' || !tunnelUrl || p.tunnelUrl === tunnelUrl) return;
    setNotice('The address changed. Scan the new QR');
    void startPairing({ via: 'tunnel' });
  }, [tunnelUrl, startPairing]);

  const run = async (action: () => Promise<PhoneStatus>, onError?: (message: string) => void) => {
    setError(null);
    try {
      apply(await action());
    } catch (e) {
      const message = phoneErrorText(e, 'That did not work.');
      if (!alive.current) return;
      if (onError) onError(message);
      else setError(message);
    }
  };

  const waitForTunnel = async (first: PhoneStatus): Promise<boolean> => {
    let current = first;
    const deadline = Date.now() + POLL_MAX_MS;
    while (alive.current) {
      if (current.tunnel.phase === 'running') return true;
      if (current.tunnel.phase === 'error' || Date.now() >= deadline) return false;
      await sleep(POLL_MS);
      try {
        current = await api.status();
        apply(current);
      } catch {
        return false;
      }
    }
    return false;
  };

  const connectAnywhere = async () => {
    setBusy(true);
    setError(null);
    try {
      const first = await api.tunnelSet(true);
      apply(first);
      if (await waitForTunnel(first)) await startPairing({ via: 'tunnel' });
      else if (alive.current) setError('The tunnel did not start in time. Try again in a moment.');
    } catch (e) {
      if (alive.current) setError(phoneErrorText(e, 'Could not turn on the tunnel.'));
    } finally {
      if (alive.current) setBusy(false);
    }
  };

  if (!status) {
    return error ? <Note role="alert">{error}</Note> : <p className="st-note">Loading…</p>;
  }

  const ways = availableWays(status);
  const way = wayPick && ways.some((w) => w.via === wayPick) ? wayPick : defaultWay(status);
  const addresses = [...(status.customAddress ? [status.customAddress] : []), ...status.lanUrls, ...(status.tunnel.url ? [status.tunnel.url] : [])];
  const secsLeft = pairing ? (pairing.expiresAtMs - now) / 1000 : 0;

  const statusLine = !status.enabled
    ? 'The phone server is off.'
    : status.portError
      ? status.portError
      : status.devices.length === 0
        ? 'Ready. No phone is paired yet.'
        : `Ready. ${status.devices.length} ${status.devices.length === 1 ? 'phone is' : 'phones are'} paired.`;

  const askRevoke = (d: PhoneDevice) =>
    setAsk({
      title: `Sign out ${d.name}?`,
      body: 'This phone will be signed out at once.',
      confirmLabel: 'Revoke',
      run: () => void run(() => api.deviceRevoke(d.id)),
    });

  const askTunnel = (then: () => void) => setAsk({ title: 'Reach your PC from outside your Wi-Fi?', body: TUNNEL_SENTENCE, confirmLabel: 'Turn on', run: then });

  return (
    <div className="st-phone">
      {ask && (
        <ConfirmSheet
          title={ask.title}
          body={ask.body}
          confirmLabel={ask.confirmLabel}
          onCancel={() => setAsk(null)}
          onConfirm={() => {
            const go = ask.run;
            setAsk(null);
            go();
          }}
        />
      )}

      <SettingBlock label="Use your bots from your phone" description={statusLine}>
        {addresses.length > 0 ? (
          <ul aria-label="Phone addresses" style={{ margin: 0, paddingLeft: 16 }}>
            {addresses.map((a) => (
              <li key={a} className="st-value">
                {a}
              </li>
            ))}
          </ul>
        ) : (
          <p className="st-note">No phone address yet. Join a Wi-Fi network, or use "Reach from outside your Wi-Fi" below.</p>
        )}
      </SettingBlock>

      <SettingBlock label="Pair a phone" description="Scan the QR with your phone camera.">
        {ways.length > 1 && (
          <div>
            <FieldLabel>How the phone will reach this PC</FieldLabel>
            <FieldSelect aria-label="How the phone will reach this PC" value={way} onChange={(e) => setWayPick(e.target.value as PairVia)}>
              {ways.map((w) => (
                <option key={w.via} value={w.via}>
                  {w.label}
                </option>
              ))}
            </FieldSelect>
          </div>
        )}
        <div>
          <Btn primary onClick={() => void startPairing({ via: way })}>
            {pairing ? 'Make a new code' : 'Show pairing QR'}
          </Btn>
        </div>
        {pairing && (
          <div aria-label="Pairing">
            <img src={pairing.qr} alt="QR code to pair your phone" width={240} height={240} style={{ display: 'block', background: 'white', borderRadius: 8 }} />
            <p className="st-value" aria-label="Pairing code" style={{ fontSize: 20, letterSpacing: 2 }}>
              {groupCode(pairing.code)}
            </p>
            <p className="st-note" role="status">
              {secsLeft > 0 ? `Works once, for 10 minutes. Time left ${countdownText(secsLeft)}.` : 'This code ran out. Make a new code.'}
            </p>
            {notice && <Note role="status">{notice}</Note>}
            <ol className="st-note" style={{ margin: 0, paddingLeft: 18 }}>
              <li>Scan this with your phone camera.</li>
              <li>Add it to your home screen.</li>
            </ol>
          </div>
        )}
      </SettingBlock>

      <SettingBlock label="Phones" description="The phones that can use your bots.">
        {status.devices.length === 0 ? (
          <p className="st-note">No phones yet.</p>
        ) : (
          <ul aria-label="Paired phones" style={{ margin: 0, padding: 0, listStyle: 'none' }}>
            {status.devices.map((d) => (
              <li key={d.id} data-device={d.id} style={{ display: 'flex', gap: 12, alignItems: 'center', justifyContent: 'space-between', padding: '6px 0' }}>
                <span>
                  <span className="st-value">{d.name}</span>
                  <small style={{ display: 'block' }}>
                    {pairedText(d.pairedAtSecs)} · {lastUsedText(d.lastSeenSecs, now)}
                    {d.expired ? ' · Needs a new link' : ''}
                  </small>
                </span>
                <span style={{ display: 'flex', gap: 6 }}>
                  <Btn small title="Use when your address changed. The phone keeps its name and settings." aria-label={`Re-link ${d.name}`} onClick={() => void startPairing({ via: way, relinkDeviceId: d.id })}>
                    Re-link
                  </Btn>
                  <Btn small aria-label={`Revoke ${d.name}`} onClick={() => askRevoke(d)}>
                    Revoke
                  </Btn>
                </span>
              </li>
            ))}
          </ul>
        )}
        {status.devices.length > 0 && <p className="st-note">Re-link: use when your address changed. The phone keeps its name and settings.</p>}
      </SettingBlock>

      <SettingBlock label="Can't connect?" description="Some Wi-Fi networks (guest, mesh, hotspot) stop phones from reaching your PC. Windows Firewall can block it too.">
        {status.firewall.state === 'missing' && (
          <div>
            <p className="st-note">{status.firewall.message}</p>
            <p className="st-note">This only opens private networks, and only this port. Windows will ask you to confirm.</p>
            <Btn onClick={() => void run(() => api.firewallAllow())}>Allow in Windows Firewall</Btn>
          </div>
        )}
        <div>
          <Btn disabled={busy} onClick={() => askTunnel(() => void connectAnywhere())}>
            {busy ? 'Starting the tunnel…' : "Can't connect? Turn on and show a new QR"}
          </Btn>
        </div>
      </SettingBlock>

      <SettingBlock label="Reach from outside your Wi-Fi" description="Lets a paired phone connect from anywhere.">
        <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <Switch
            label="Reach from outside your Wi-Fi"
            checked={status.tunnel.on}
            onChange={(next) => {
              if (next) askTunnel(() => void run(() => api.tunnelSet(true)));
              else void run(() => api.tunnelSet(false));
            }}
          />
          <span className="st-value">{TUNNEL_PHASE_TEXT[status.tunnel.phase] ?? ''}</span>
        </div>
        {status.tunnel.on && status.tunnel.url && <p className="st-value">{status.tunnel.url}</p>}
        {status.tunnel.phase === 'error' && status.tunnel.error && <Note role="alert">{phoneErrorText(status.tunnel.error, 'The tunnel stopped.')}</Note>}
      </SettingBlock>

      <details>
        <summary>Advanced</summary>
        <SettingBlock label="Port" description="Only the phone server restarts; the tunnel follows.">
          <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            <FieldInput aria-label="Port" type="number" min={1024} max={65535} value={portText} onChange={(e) => setPortText(e.target.value)} />
            <Btn
              onClick={() => {
                setPortMsg(null);
                const n = Number(portText);
                if (!Number.isInteger(n) || n < 1024 || n > 65535) {
                  setPortMsg('Pick a number from 1024 to 65535.');
                  return;
                }
                void run(() => api.setPort(n), setPortMsg);
              }}
            >
              Save port
            </Btn>
          </div>
          {(portMsg || status.portError) && <Note role="alert">{portMsg ?? status.portError}</Note>}
        </SettingBlock>
        <SettingBlock
          label="My own address"
          description={`For a reverse proxy or a named Cloudflare tunnel in front of port ${status.port}. Example: https://bot.example.com. It is tried before the tunnel.`}
        >
          <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            <FieldInput aria-label="My own address" type="text" placeholder="https://bot.example.com" value={addrText} onChange={(e) => setAddrText(e.target.value)} />
            <Btn
              onClick={() => {
                setAddrMsg(null);
                void run(() => api.setCustomAddress(addrText.trim() || null), setAddrMsg);
              }}
            >
              Save address
            </Btn>
            <Btn
              onClick={() => {
                setAddrMsg(null);
                setAddrText('');
                void run(() => api.setCustomAddress(null), setAddrMsg);
              }}
            >
              Clear
            </Btn>
          </div>
          {addrMsg && <Note role="alert">{addrMsg}</Note>}
          {status.customWarning && (
            <Note role="alert">
              <strong>{status.customWarning}</strong>
            </Note>
          )}
        </SettingBlock>
      </details>

      {error && <Note role="alert">{error}</Note>}
    </div>
  );
}
