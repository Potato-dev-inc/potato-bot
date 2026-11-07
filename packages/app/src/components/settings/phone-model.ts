// Small pure helpers for Settings > Phone: the QR picture, the code in groups, times in words, which way to pair.
import QRCode from 'qrcode';
import type { PairVia, PhoneStatus } from '../../lib/phone-api';

/** The QR as an SVG data URL (black squares on a white page with a quiet border), drawn from the code's own matrix. */
export function qrDataUrl(text: string): string {
  const qr = QRCode.create(text, { errorCorrectionLevel: 'M' });
  const size = qr.modules.size;
  const quiet = 4;
  const total = size + quiet * 2;
  let path = '';
  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      if (qr.modules.get(x, y)) path += `M${x + quiet} ${y + quiet}h1v1h-1z`;
    }
  }
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${total} ${total}" shape-rendering="crispEdges"><rect width="${total}" height="${total}" fill="white"/><path d="${path}" fill="black"/></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

/** "ABCD1234EFGH" -> "ABCD 1234 EFGH". */
export function groupCode(code: string): string {
  return (code.replace(/\s+/g, '').match(/.{1,4}/g) ?? []).join(' ');
}

export function countdownText(secsLeft: number): string {
  const s = Math.max(0, Math.floor(secsLeft));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

export function pairedText(secs: number): string {
  return `Paired ${new Date(secs * 1000).toLocaleDateString()}`;
}

/** "Last used 5 minutes ago" or "Not used yet". */
export function lastUsedText(secs: number | null, nowMs: number): string {
  if (secs === null) return 'Not used yet';
  const diff = Math.max(0, Math.floor(nowMs / 1000 - secs));
  if (diff < 60) return 'Last used just now';
  const unit = (n: number, word: string) => `Last used ${n} ${word}${n === 1 ? '' : 's'} ago`;
  if (diff < 3600) return unit(Math.floor(diff / 60), 'minute');
  if (diff < 86400) return unit(Math.floor(diff / 3600), 'hour');
  return unit(Math.floor(diff / 86400), 'day');
}

export interface WayOption {
  via: PairVia;
  label: string;
}

/** The ways a phone can reach this PC right now. */
export function availableWays(status: PhoneStatus): WayOption[] {
  const ways: WayOption[] = [];
  if (status.lanUrls.length > 0) ways.push({ via: 'lan', label: 'Same Wi-Fi' });
  if (status.customAddress) ways.push({ via: 'custom', label: 'My own address' });
  if (status.tunnel.phase === 'running' && status.tunnel.url) ways.push({ via: 'tunnel', label: 'Anywhere' });
  return ways;
}

/** The way to use when the person has not picked one: Same Wi-Fi if there is one, else my own address, else the tunnel. */
export function defaultWay(status: PhoneStatus): PairVia {
  const ways = availableWays(status);
  return ways[0]?.via ?? 'lan';
}
