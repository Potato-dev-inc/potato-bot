import { useEffect, type RefObject } from 'react';

/** A small menu: Escape closes it and gives focus back to its button; a click outside closes it too. */
export function useMenuDismiss(open: boolean, box: RefObject<HTMLElement | null>, button: RefObject<HTMLElement | null>, close: () => void): void {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopPropagation();
      close();
      button.current?.focus();
    };
    const onDown = (e: MouseEvent) => {
      if (e.target instanceof Node && !box.current?.contains(e.target)) close();
    };
    document.addEventListener('keydown', onKey, true);
    document.addEventListener('mousedown', onDown);
    return () => { document.removeEventListener('keydown', onKey, true); document.removeEventListener('mousedown', onDown); };
  }, [open, box, button, close]);
}
