// Typed wrappers for the phone feature's Tauri commands (camelCase JSON). Plain TypeScript: no React and no DOM,
// so a future React Native app can reuse the types. The caller passes the `invoke` and `listen` functions.
// Errors are plain strings `CODE: plain sentence`; use `phoneErrorText` to show the sentence.

export interface PhoneDevice {
  id: string;
  name: string;
  pairedAtSecs: number;
  lastSeenSecs: number | null;
  expired: boolean;
  caps: string[];
  /** The PC's saved yes to this phone using the mouse and keyboard on the bots' computer. */
  controlComputer?: boolean;
}

export type TunnelPhase = 'off' | 'starting' | 'running' | 'reconnecting' | 'error';
export type FirewallState = 'ok' | 'missing' | 'unknown' | 'not_windows';
export type PairVia = 'lan' | 'custom' | 'tunnel';

export interface PhoneStatus {
  enabled: boolean;
  port: number;
  portError: string | null;
  pcName: string;
  lanUrls: string[];
  customAddress: string | null;
  customWarning: string | null;
  tunnel: { on: boolean; phase: TunnelPhase; url: string | null; error: string | null };
  firewall: { state: FirewallState; message: string };
  devices: PhoneDevice[];
}

export interface PairingStart {
  code: string;
  expiresAtSecs: number;
  /** What the QR encodes. One-time: never log it or store it. */
  qrUrl: string;
  candidates: string[];
  via: string;
}

export type PhoneInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export const PHONE_CHANGED_EVENT = 'phone://changed';

/** `CODE: A plain sentence.` -> the sentence only. */
export function phoneErrorText(error: unknown, fallback: string): string {
  const raw = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  const text = raw.replace(/^[A-Z][A-Z0-9_]{2,}:\s*/, '').trim();
  return text || fallback;
}

export interface PhoneApi {
  status(): Promise<PhoneStatus>;
  pairingStart(opts?: { via?: PairVia; relinkDeviceId?: string }): Promise<PairingStart>;
  deviceRevoke(deviceId: string): Promise<PhoneStatus>;
  /** Only the PC can say this; the phone API has no route for it. */
  deviceControlSet(deviceId: string, on: boolean): Promise<PhoneStatus>;
  setPort(port: number): Promise<PhoneStatus>;
  setCustomAddress(address: string | null): Promise<PhoneStatus>;
  tunnelSet(on: boolean): Promise<PhoneStatus>;
  firewallAllow(): Promise<PhoneStatus>;
}

export function makePhoneApi(invoke: PhoneInvoke): PhoneApi {
  return {
    status: () => invoke<PhoneStatus>('phone_status'),
    pairingStart: (opts) => invoke<PairingStart>('phone_pairing_start', { via: opts?.via, relinkDeviceId: opts?.relinkDeviceId }),
    deviceRevoke: (deviceId) => invoke<PhoneStatus>('phone_device_revoke', { deviceId }),
    deviceControlSet: (deviceId, on) => invoke<PhoneStatus>('phone_device_control_set', { deviceId, on }),
    setPort: (port) => invoke<PhoneStatus>('phone_set_port', { port }),
    setCustomAddress: (address) => invoke<PhoneStatus>('phone_set_custom_address', { address }),
    tunnelSet: (on) => invoke<PhoneStatus>('phone_tunnel_set', { on }),
    firewallAllow: () => invoke<PhoneStatus>('phone_firewall_allow'),
  };
}
