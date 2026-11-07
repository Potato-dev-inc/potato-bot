// The sidebar's Spaces row (Labs `spaces_enabled`). Rendered only when the flag is on; off, it is not in the page at all.
export function SpacesRow({ enabled, open, onOpen }: { enabled: boolean; open: boolean; onOpen: () => void }) {
  if (!enabled) return null;
  return (
    <button
      type="button"
      className={`ck-row ck-row--flat${open ? ' is-active' : ''}`}
      aria-current={open ? 'page' : undefined}
      onClick={onOpen}
    >
      <span className="ck-row-glyph ck-row-glyph--icon" aria-hidden="true">S</span>
      <span className="ck-row-main">
        <span className="ck-row-title">Spaces</span>
      </span>
    </button>
  );
}
