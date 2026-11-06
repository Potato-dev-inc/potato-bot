// Two small buttons in the full-window computer while the person has control: Paste puts what they copied in Windows into
// the bot's computer, Copy puts what they copied in the computer back on their clipboard. Both are plain button presses
// (a press is what lets the app touch the clipboard), text only, and the computer's clipboard is held until Copy is
// pressed: nothing the bot or a web page copies ever replaces the person's own clipboard by itself.
import { useEffect, useState } from 'react';
import { COMPUTER_COPY } from '../../../lib/potato-computer';

export interface ComputerClipboardProps {
  /** What the computer copied while the person had control, or null. */
  remote: string | null;
  /** Sends text to the computer; false when there is no screen to send to. */
  paste: (text: string) => boolean;
}

export function ComputerClipboard({ remote, paste }: ComputerClipboardProps) {
  const [said, setSaid] = useState<string | null>(null);
  // A new copy in the computer clears an old answer.
  useEffect(() => setSaid(null), [remote]);

  const doPaste = async () => {
    let text: string | null = null;
    try {
      text = await navigator.clipboard.readText();
    } catch {
      setSaid(COMPUTER_COPY.clipboardBlocked);
      return;
    }
    if (!text) {
      setSaid(COMPUTER_COPY.clipboardEmpty);
      return;
    }
    setSaid(paste(text) ? COMPUTER_COPY.pasted : COMPUTER_COPY.clipboardTooBig);
  };
  const doCopy = async () => {
    if (remote === null) return;
    try {
      await navigator.clipboard.writeText(remote);
      setSaid(COMPUTER_COPY.copied);
    } catch {
      setSaid(COMPUTER_COPY.clipboardBlocked);
    }
  };

  return (
    <span className="ck-potato-overlay-clip">
      <button type="button" className="ck-btn ck-btn--secondary" title={COMPUTER_COPY.pasteHint} onClick={() => void doPaste()}>
        {COMPUTER_COPY.paste}
      </button>
      <button type="button" className="ck-btn ck-btn--secondary" disabled={remote === null} title={COMPUTER_COPY.copyHint} onClick={() => void doCopy()}>
        {COMPUTER_COPY.copyOut}
      </button>
      <span className="ck-sub ck-potato-overlay-clip-note" role="status">
        {said ?? ''}
      </span>
    </span>
  );
}
