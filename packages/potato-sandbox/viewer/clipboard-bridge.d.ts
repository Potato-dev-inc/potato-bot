// Types for the tests (clipboard-bridge.js is plain JavaScript run in the viewer page). Not served to the browser.
export const PASTE_MESSAGE_TYPE: string;
export const CLIPBOARD_MESSAGE_TYPE: string;
export const MAX_CLIPBOARD_CHARS: number;
export interface RfbLike {
  viewOnly?: boolean;
  clipboardPasteFrom?: (text: string) => void;
  sendKey?: (keysym: number, code: string, down?: boolean) => void;
  addEventListener?: (type: string, listener: (event: any) => void) => void;
  removeEventListener?: (type: string, listener: (event: any) => void) => void;
  _rfbConnectionState?: string;
}
export function isPasteChord(event: { ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; code?: string; key?: string }): boolean;
export function pasteHostText(rfb: RfbLike, text: string): boolean;
export function attachRemoteClipboardCopy(rfb: RfbLike, options?: { clipboard?: { writeText?: (text: string) => Promise<void> } | null }): () => void;
export function attachParentClipboard(rfb: RfbLike, options?: { parentOrigins?: string[]; window?: unknown; parent?: unknown }): () => void;
