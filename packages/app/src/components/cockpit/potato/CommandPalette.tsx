// The Ctrl+K palette on the Potato page: a box that filters the bots, groups and pages, and runs the one picked.
// Up and Down move the highlight, Enter runs it, Escape closes. Grayscale: the same tokens as the rest of the page.
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { filterPalette, nextPaletteIndex, PALETTE_COPY, type PaletteItem } from '../../../lib/potato-palette';

export interface CommandPaletteProps {
  items: readonly PaletteItem[];
  onClose: () => void;
}

export function CommandPalette({ items, onClose }: CommandPaletteProps) {
  const [query, setQuery] = useState('');
  const [index, setIndex] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const matches = useMemo(() => filterPalette(items, query), [items, query]);

  useEffect(() => {
    input.current?.focus();
  }, []);
  useEffect(() => {
    setIndex(matches.length > 0 ? 0 : -1);
  }, [matches.length, query]);

  const runAt = (i: number) => {
    const item = matches[i];
    if (!item) return;
    onClose();
    item.run();
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      setIndex((i) => nextPaletteIndex(i, matches.length, e.key === 'ArrowDown' ? 1 : -1));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      runAt(index);
    }
  };

  return (
    <div className="ck-potato-palette-backdrop" onMouseDown={onClose}>
      <div role="dialog" aria-modal="true" aria-label={PALETTE_COPY.title} className="ck-potato-palette" onMouseDown={(e) => e.stopPropagation()} onKeyDown={onKey}>
        <input
          ref={input}
          type="text"
          className="ck-potato-palette-input"
          placeholder={PALETTE_COPY.placeholder}
          aria-label={PALETTE_COPY.placeholder}
          aria-controls="ck-potato-palette-list"
          aria-activedescendant={index >= 0 ? `ck-potato-palette-${index}` : undefined}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <ul id="ck-potato-palette-list" role="listbox" className="ck-potato-palette-list">
          {matches.length === 0 && <li className="ck-potato-palette-empty">{PALETTE_COPY.empty}</li>}
          {matches.map((item, i) => (
            <li
              key={item.id}
              id={`ck-potato-palette-${i}`}
              role="option"
              aria-selected={i === index}
              className="ck-potato-palette-item"
              data-active={i === index || undefined}
              onMouseEnter={() => setIndex(i)}
              onClick={() => runAt(i)}
            >
              <span className="ck-potato-palette-label">{item.label}</span>
              <span className="ck-potato-palette-hint">{item.hint}</span>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
