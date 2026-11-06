// The command palette (Ctrl+K on the Potato page): one list of places to jump to, filtered by what the person types.
// Pure: the page builds the items (each with the action it runs); this only filters them.

export interface PaletteItem {
  id: string;
  /** What the row says: a bot's name, a group's name, or a page's name. */
  label: string;
  /** A short grey word after the label: "Bot", "Group", "Page". */
  hint: string;
  run: () => void;
}

export const PALETTE_COPY = {
  title: 'Jump to',
  placeholder: 'Bot, group or page',
  empty: 'Nothing matches',
  close: 'Close',
} as const;

/** The items whose label or hint contains the query (case-insensitive). An empty query keeps them all, in order. */
export function filterPalette(items: readonly PaletteItem[], query: string): PaletteItem[] {
  const q = query.trim().toLowerCase();
  if (q === '') return [...items];
  return items.filter((item) => `${item.label} ${item.hint}`.toLowerCase().includes(q));
}

/** The row the highlight moves to: wraps around at both ends. -1 when there is nothing to pick. */
export function nextPaletteIndex(current: number, count: number, step: 1 | -1): number {
  if (count <= 0) return -1;
  if (current < 0) return step === 1 ? 0 : count - 1;
  return (current + step + count) % count;
}
