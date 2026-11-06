/*
Ported from Rakazo (https://github.com/elie222/rakazo), commit f48fa5c7, file infra/sandboxes/computer/clipboard-bridge.js.
Copyright 2026 Rakazo contributors. Licensed under the Apache License, Version 2.0;
see packages/potato-sandbox/LICENSE-APACHE-2.0. Changed by Puppet Master: added attachParentClipboard (the app's own
Paste / Copy buttons talk to this page by postMessage; only the allowlisted parent window, only while the person has control,
text only, 100000 characters at most). The rest of the file is unchanged.
*/

/**
 * Clipboard bridge for the chrome-less noVNC embed.
 *
 * Host → sandbox: Debian noVNC 1.3 only syncs clipboard from the full
 * vnc.html panel. Without this bridge, Ctrl/Cmd+V only forwarded keys: on
 * macOS Cmd+V became Super+V (no paste on Linux), and the remote CLIPBOARD
 * stayed empty.
 * Sandbox → host: noVNC reports remote CLIPBOARD changes as a "clipboard"
 * event, but nothing writes them to the host clipboard without a listener.
 */

export const KEYSYM = {
  Control_L: 0xffe3,
  Control_R: 0xffe4,
  Alt_L: 0xffe9,
  Alt_R: 0xffea,
  Super_L: 0xffeb,
  Super_R: 0xffec,
  Mode_switch: 0xff7e,
  ISO_Level3_Shift: 0xfe03,
  v: 0x76,
};

/** @param {{ ctrlKey?: boolean, metaKey?: boolean, altKey?: boolean, code?: string, key?: string }} event */
export function isPasteChord(event) {
  if (!event || event.altKey) return false;
  if (!(event.ctrlKey || event.metaKey)) return false;
  return event.code === "KeyV" || event.key === "v" || event.key === "V";
}

/** @param {{ clipboardData?: DataTransfer | null }} event */
export function clipboardTextFromPaste(event) {
  const data = event?.clipboardData;
  if (!data) return "";
  return data.getData("text/plain") || data.getData("text") || "";
}

/** True when noVNC RFB can accept clipboard and key events. */
export function isRfbConnected(rfb) {
  const state = rfb?._rfbConnectionState;
  return !state || state === "connected";
}

/**
 * Release modifiers that may still be down from the host paste chord.
 * noVNC 1.3 remaps macOS MetaLeft→Alt_L and MetaRight→Super_L (and Alt→
 * Mode_switch / ISO_Level3_Shift), so release those keysyms too.
 * @param {(keysym: number, code: string, down?: boolean) => void} sendKey
 */
export function releaseModifierKeys(sendKey) {
  sendKey(KEYSYM.Control_L, "ControlLeft", false);
  sendKey(KEYSYM.Control_R, "ControlRight", false);
  sendKey(KEYSYM.Alt_L, "AltLeft", false);
  sendKey(KEYSYM.Alt_R, "AltRight", false);
  sendKey(KEYSYM.Super_L, "MetaLeft", false);
  sendKey(KEYSYM.Super_R, "MetaRight", false);
  sendKey(KEYSYM.Mode_switch, "AltLeft", false);
  sendKey(KEYSYM.ISO_Level3_Shift, "AltRight", false);
}

/** Send Linux Ctrl+V so the focused remote app pastes from CLIPBOARD. */
export function sendRemotePaste(sendKey) {
  sendKey(KEYSYM.Control_L, "ControlLeft", true);
  sendKey(KEYSYM.v, "KeyV", true);
  sendKey(KEYSYM.v, "KeyV", false);
  sendKey(KEYSYM.Control_L, "ControlLeft", false);
}

/**
 * Sync host paste into the RFB session, then paste into the remote desktop.
 * @param {{ viewOnly?: boolean, clipboardPasteFrom?: (text: string) => void, sendKey?: Function, _rfbConnectionState?: string }} rfb
 * @param {string} text
 */
export function pasteHostText(rfb, text) {
  if (!rfb || rfb.viewOnly || !text || !isRfbConnected(rfb)) return false;
  const sendKey = rfb.sendKey?.bind(rfb);
  if (!sendKey || typeof rfb.clipboardPasteFrom !== "function") return false;
  releaseModifierKeys(sendKey);
  rfb.clipboardPasteFrom(text);
  sendRemotePaste(sendKey);
  return true;
}

/**
 * Read plain text from the host clipboard API.
 * Returns null when the API is missing or denied so callers can fall back to a
 * focusable paste target. An empty clipboard is "".
 * @param {{ readText?: () => Promise<string> } | null | undefined} [clipboard]
 */
export async function readHostClipboardText(clipboard = globalThis.navigator?.clipboard) {
  if (!clipboard || typeof clipboard.readText !== "function") return null;
  try {
    return (await clipboard.readText()) || "";
  } catch {
    return null;
  }
}

/**
 * Focus a paste fallback during the tap and park the caret after any sentinel
 * so OS paste cannot treat those characters as clipboard text.
 * @param {{ focus?: () => void, value?: string, setSelectionRange?: (start: number, end: number) => void } | null | undefined} target
 */
function focusPasteTarget(target) {
  if (!target || typeof target.focus !== "function") return;
  target.focus();
  if (typeof target.value !== "string") return;
  const length = target.value.length;
  target.setSelectionRange?.(length, length);
}

/**
 * Touch Paste control: one tap reads the host clipboard and pastes through the
 * RFB bridge. When the clipboard API is unavailable, focus a paste target so
 * the existing `paste` listener can fire without a modifier chord.
 * @param {{ viewOnly?: boolean, clipboardPasteFrom?: (text: string) => void, sendKey?: Function, _rfbConnectionState?: string }} rfb
 * @param {{ button?: HTMLElement | null, clipboard?: { readText?: () => Promise<string> }, fallbackFocus?: { focus?: () => void, blur?: () => void, value?: string, setSelectionRange?: (start: number, end: number) => void } | null }} [options]
 * @returns {() => void} detach
 */
export function attachMobilePaste(rfb, options = {}) {
  const button = options.button;
  if (!button || rfb.viewOnly) return () => {};
  button.hidden = false;
  const onClick = async () => {
    const clipboard = options.clipboard ?? globalThis.navigator?.clipboard;
    const fallback = options.fallbackFocus;
    // Focus during the tap, before any await, so a denial still has a software
    // keyboard and a caret parked after the sentinel.
    focusPasteTarget(fallback);
    const text = await readHostClipboardText(clipboard);
    if (text) {
      pasteHostText(rfb, text);
      fallback?.blur?.();
      return;
    }
    if (text === "") fallback?.blur?.();
  };
  button.addEventListener("click", onClick);
  return () => {
    button.removeEventListener("click", onClick);
  };
}

/**
 * Sandbox → host copy: Debian noVNC 1.3 dispatches a "clipboard" CustomEvent
 * with detail.text when x11vnc reports a remote CLIPBOARD change
 * (ServerCutText). navigator.clipboard.writeText needs a secure context and
 * can be denied without a gesture, so failures are silent.
 * @param {{ addEventListener?: (type: string, listener: (event: object) => void) => void, removeEventListener?: (type: string, listener: (event: object) => void) => void } | null | undefined} rfb
 * @param {{ clipboard?: { writeText?: (text: string) => Promise<void> } | null }} [options]
 * @returns {() => void} detach
 */
export function attachRemoteClipboardCopy(rfb, options = {}) {
  const clipboard = options.clipboard ?? globalThis.navigator?.clipboard;
  if (typeof rfb?.addEventListener !== "function") return () => {};
  if (!clipboard || typeof clipboard.writeText !== "function") return () => {};
  const onClipboard = async (event) => {
    const text = event?.detail?.text;
    if (typeof text !== "string") return;
    try {
      await clipboard.writeText(text);
    } catch {
      // Clipboard writes can be denied without a gesture; stay silent.
    }
  };
  rfb.addEventListener("clipboard", onClipboard);
  return () => {
    rfb.removeEventListener?.("clipboard", onClipboard);
  };
}

/**
 * @param {{ viewOnly?: boolean, clipboardPasteFrom?: (text: string) => void, sendKey?: Function, _rfbConnectionState?: string }} rfb
 * @param {{ target?: EventTarget }} [options]
 * @returns {() => void} detach
 */
export function attachHostClipboardPaste(rfb, options = {}) {
  const target = options.target ?? globalThis;
  const onKeyDown = (event) => {
    if (rfb.viewOnly || !isPasteChord(event)) return;
    // Keep default so the browser still fires `paste` with clipboardData.
    event.stopPropagation();
  };
  const onKeyUp = (event) => {
    if (rfb.viewOnly || !isPasteChord(event)) return;
    event.stopPropagation();
  };
  const onPaste = (event) => {
    if (rfb.viewOnly) return;
    const text = clipboardTextFromPaste(event);
    if (!text) return;
    if (!pasteHostText(rfb, text)) return;
    event.preventDefault();
    event.stopPropagation();
  };

  target.addEventListener("keydown", onKeyDown, true);
  target.addEventListener("keyup", onKeyUp, true);
  target.addEventListener("paste", onPaste, true);
  return () => {
    target.removeEventListener("keydown", onKeyDown, true);
    target.removeEventListener("keyup", onKeyUp, true);
    target.removeEventListener("paste", onPaste, true);
  };
}

/** The message the app sends to paste its text into the remote desktop. */
export const PASTE_MESSAGE_TYPE = "pm-sbx-paste";
/** The message this page sends to the app when the remote clipboard changed. */
export const CLIPBOARD_MESSAGE_TYPE = "pm-sbx-clipboard";
/** The most characters that cross either way. Longer text is dropped, never cut. */
export const MAX_CLIPBOARD_CHARS = 100000;

/**
 * The app's Paste and Copy buttons. Only while the person has control (a view-only page attaches nothing).
 * - App to computer: a `pm-sbx-paste` message with `text`. Believed only from the parent window and an allowlisted origin.
 * - Computer to app: when the remote clipboard changes, a `pm-sbx-clipboard` message with `text` goes to the allowlisted
 *   origins only. The app holds it until the person presses Copy; this message writes nothing to their clipboard.
 * @param {{ viewOnly?: boolean, clipboardPasteFrom?: Function, sendKey?: Function, addEventListener?: Function, removeEventListener?: Function, _rfbConnectionState?: string }} rfb
 * @param {{ parentOrigins?: string[], window?: any, parent?: any }} [options]
 * @returns {() => void} detach
 */
export function attachParentClipboard(rfb, options = {}) {
  const win = options.window ?? globalThis;
  const parent = options.parent ?? win.parent;
  const origins = Array.isArray(options.parentOrigins) ? options.parentOrigins.filter((o) => typeof o === "string" && o !== "*") : [];
  if (!rfb || rfb.viewOnly || !parent || parent === win || origins.length === 0) return () => {};
  const onMessage = (event) => {
    if (rfb.viewOnly) return;
    if (!event || event.source !== parent || !origins.includes(event.origin)) return;
    const data = event.data;
    if (!data || typeof data !== "object" || data.type !== PASTE_MESSAGE_TYPE) return;
    const text = data.text;
    if (typeof text !== "string" || text === "" || text.length > MAX_CLIPBOARD_CHARS) return;
    pasteHostText(rfb, text);
  };
  const onClipboard = (event) => {
    if (rfb.viewOnly) return;
    const text = event?.detail?.text;
    if (typeof text !== "string" || text === "" || text.length > MAX_CLIPBOARD_CHARS) return;
    for (const origin of origins) parent.postMessage({ type: CLIPBOARD_MESSAGE_TYPE, text }, origin);
  };
  win.addEventListener?.("message", onMessage);
  rfb.addEventListener?.("clipboard", onClipboard);
  return () => {
    win.removeEventListener?.("message", onMessage);
    rfb.removeEventListener?.("clipboard", onClipboard);
  };
}
