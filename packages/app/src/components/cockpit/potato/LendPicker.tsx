// The picker behind "Lend an open app": the open windows on this PC with a thumbnail each, a Show it / Keep it out of my
// way choice, and Lend / Cancel. Known chat apps are listed first, the rest under "Other apps". Elevated windows cannot
// be picked and sit behind a "Show N more" button. The list is read again every 2 s while the picker is open. Esc closes
// it. Nothing here takes focus from the app the person is using; the thumbnails are read on their own, one row at a time.
import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { LEND_COPY } from '../../../lib/potato-computer';
import { lendErrorText, type LendMode, type LendTransport, type WinInfo } from '../../../lib/potatoLend';

/** How often the candidate list is read again while the picker is open. */
export const PICKER_REFRESH_MS = 2000;

/** Apps people chat in: listed first. Matched against the app name, case-insensitive. */
const CHAT_APPS = ['line', 'whatsapp', 'telegram', 'messenger', 'discord', 'signal', 'wechat', 'slack'];

export function isChatApp(win: WinInfo): boolean {
  const name = win.app_name.toLowerCase();
  return CHAT_APPS.some((word) => name.includes(word));
}

export interface LendPickerProps {
  botName: string;
  transport: LendTransport;
  onClose: () => void;
  /** The app is lent: the caller reads the list again. */
  onLent: () => void;
}

export function LendPicker({ botName, transport, onClose, onLent }: LendPickerProps) {
  const [windows, setWindows] = useState<WinInfo[] | null>(null);
  const [picked, setPicked] = useState<number | null>(null);
  const [mode, setMode] = useState<LendMode>('visible');
  const [showElevated, setShowElevated] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Read the list now, then every PICKER_REFRESH_MS. The timer stops when the picker closes (it unmounts).
  useEffect(() => {
    let live = true;
    let inFlight = false;
    const load = () => {
      if (inFlight) return;
      inFlight = true;
      transport
        .candidates()
        .then(
          (list) => {
            if (!live) return;
            setWindows(list);
            // A picked window that closed, or turned elevated, is no longer picked.
            setPicked((p) => (p !== null && list.some((w) => w.hwnd === p && !w.elevated) ? p : null));
          },
          (e: unknown) => {
            if (!live) return;
            setWindows((prev) => prev ?? []);
            setError(lendErrorText(e));
          },
        )
        .finally(() => {
          inFlight = false;
        });
    };
    load();
    const timer = setInterval(load, PICKER_REFRESH_MS);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [transport]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  const lend = async () => {
    if (picked === null || busy) return;
    setBusy(true);
    setError(null);
    try {
      await transport.start(picked, mode);
      onLent();
    } catch (e) {
      setError(lendErrorText(e));
      setBusy(false);
    }
  };

  const all = windows ?? [];
  const pickedName = all.find((w) => w.hwnd === picked)?.app_name ?? null;
  const elevated = all.filter((w) => w.elevated);
  const chat = all.filter((w) => !w.elevated && isChatApp(w));
  const others = [...all.filter((w) => !w.elevated && !isChatApp(w)), ...(showElevated ? elevated : [])];

  const choice = (win: WinInfo) => (
    <li key={win.hwnd}>
      <label className={`ck-potato-lend-choice${win.elevated ? ' is-disabled' : ''}`}>
        <input
          type="radio"
          name="ck-potato-lend-app"
          value={win.hwnd}
          disabled={win.elevated}
          checked={picked === win.hwnd}
          onChange={() => setPicked(win.hwnd)}
        />
        <LendThumb hwnd={win.hwnd} transport={transport} alt={win.app_name} />
        <span className="ck-potato-lend-choice-text">
          <span className="ck-potato-lend-name">{win.app_name}</span>
          <span className="ck-sub">{win.elevated ? LEND_COPY.elevated : win.title}</span>
        </span>
      </label>
    </li>
  );

  return createPortal(
    <div className="ck-potato-lend-overlay" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div className="ck-potato-lend-dialog" role="dialog" aria-modal="true" aria-labelledby="ck-potato-lend-picker-title">
        <h3 id="ck-potato-lend-picker-title" className="ck-potato-lend-title">
          {LEND_COPY.pickerTitle}
        </h3>
        <p className="ck-sub ck-potato-lend-intro">{LEND_COPY.pickerIntro}</p>
        {windows === null && <p className="ck-sub">{LEND_COPY.loading}</p>}
        {windows !== null && all.length === 0 && !error && <p className="ck-sub">{LEND_COPY.noWindows}</p>}
        <ul className="ck-potato-lend-choices" role="radiogroup" aria-label={LEND_COPY.pickerTitle}>
          {chat.map(choice)}
          {others.length > 0 && (
            <li role="presentation" className="ck-potato-lend-group">
              <span className="ck-sub">{LEND_COPY.otherApps}</span>
            </li>
          )}
          {others.map(choice)}
          {!showElevated && elevated.length > 0 && (
            <li>
              <button type="button" className="ck-btn ck-btn--plain" onClick={() => setShowElevated(true)}>
                {LEND_COPY.showElevated(elevated.length)}
              </button>
            </li>
          )}
        </ul>
        <fieldset className="ck-potato-lend-modes">
          <legend className="ck-sub">{LEND_COPY.modeLabel}</legend>
          <label className="ck-potato-lend-mode">
            <input type="radio" name="ck-potato-lend-mode" checked={mode === 'visible'} onChange={() => setMode('visible')} />
            <span>{LEND_COPY.visible}</span>
            <span className="ck-sub">{LEND_COPY.visibleHint}</span>
          </label>
          <label className="ck-potato-lend-mode">
            <input type="radio" name="ck-potato-lend-mode" checked={mode === 'hidden'} onChange={() => setMode('hidden')} />
            <span>{LEND_COPY.hidden}</span>
            <span className="ck-sub">{LEND_COPY.hiddenHint}</span>
          </label>
        </fieldset>
        <p className="ck-sub ck-potato-lend-willuse">{LEND_COPY.willUse(botName, pickedName)}</p>
        {error && (
          <p role="alert" className="ck-potato-lend-error">
            {error}
          </p>
        )}
        <div className="ck-potato-maint-buttons">
          <button type="button" className="ck-btn ck-btn--primary" disabled={picked === null || busy} onClick={() => void lend()}>
            {LEND_COPY.lend(pickedName)}
          </button>
          <button type="button" className="ck-btn ck-btn--plain" onClick={onClose}>
            {LEND_COPY.cancel}
          </button>
        </div>
      </div>
    </div>,
    document.querySelector('.cockpit') ?? document.body,
  );
}

/** One window's thumbnail, read when its row first comes into view (at once where the browser has no observer). */
function LendThumb({ hwnd, transport, alt }: { hwnd: number; transport: LendTransport; alt: string }) {
  const [src, setSrc] = useState<string | null>(null);
  const [node, setNode] = useState<HTMLSpanElement | null>(null);
  useEffect(() => {
    if (!node) return;
    let live = true;
    const load = () => {
      transport.thumbnail(hwnd).then(
        (png) => {
          if (live) setSrc(`data:image/png;base64,${png}`);
        },
        () => undefined,
      );
    };
    if (typeof IntersectionObserver === 'undefined') {
      load();
      return () => {
        live = false;
      };
    }
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) {
        observer.disconnect();
        load();
      }
    });
    observer.observe(node);
    return () => {
      live = false;
      observer.disconnect();
    };
  }, [hwnd, transport, node]);
  return (
    <span className="ck-potato-lend-thumb" ref={setNode}>
      {src ? <img src={src} alt={alt} /> : null}
    </span>
  );
}
