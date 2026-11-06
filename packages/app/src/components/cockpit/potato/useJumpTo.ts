// "Jump to a message": scroll the message into the middle of a thread and mark it for a moment. Used by a routine's
// "Show reply" and by Search chats. The message may not be on screen yet (the thread was just opened and is still being read),
// so the jump waits for it: it runs once, as soon as the message is there, and not again for the same jump.
import { useEffect, useRef, type RefObject } from 'react';

export interface JumpTarget {
  id: string;
  /** Makes the same message jump again. */
  n: number;
}

const MARK_MS = 2200;

/**
 * @param scroller the thread's scrolling box
 * @param jumpTo the jump asked for (null: none)
 * @param ready changes when the rows change (the messages), so a message that arrives late is found
 * @param onJump called when the jump starts, so the thread stops following the bottom
 */
export function useJumpTo(scroller: RefObject<HTMLElement | null>, jumpTo: JumpTarget | null | undefined, ready: unknown, onJump: () => void): void {
  const done = useRef<string | null>(null);
  const mark = useRef<{ el: HTMLElement; timer: number } | null>(null);
  const call = useRef(onJump);
  call.current = onJump;

  useEffect(() => {
    if (!jumpTo) return;
    const key = `${jumpTo.id}#${jumpTo.n}`;
    if (done.current === key) return;
    const el = [...(scroller.current?.querySelectorAll<HTMLElement>('[data-message-id]') ?? [])].find((row) => row.dataset.messageId === jumpTo.id);
    if (!el) return;
    done.current = key;
    call.current();
    el.scrollIntoView?.({ block: 'center', behavior: window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
    if (mark.current) {
      window.clearTimeout(mark.current.timer);
      mark.current.el.classList.remove('is-jumped');
    }
    el.classList.add('is-jumped');
    mark.current = { el, timer: window.setTimeout(() => el.classList.remove('is-jumped'), MARK_MS) };
  }, [scroller, jumpTo, ready]);

  useEffect(
    () => () => {
      if (mark.current) window.clearTimeout(mark.current.timer);
    },
    [],
  );
}
