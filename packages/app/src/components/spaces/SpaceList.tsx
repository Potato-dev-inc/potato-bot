import type { SpaceSummary } from '../../lib/spaces';

export interface SpaceListProps {
  spaces: SpaceSummary[];
  selected: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
  onJoin: () => void;
}

/** The rail: one row per space, with unread count and who is online. */
export function SpaceList({ spaces, selected, onSelect, onNew, onJoin }: SpaceListProps) {
  return (
    <nav className="ck-spaces-rail" aria-label="Spaces">
      <ul>
        {spaces.map((s) => (
          <li key={s.id}>
            <button
              type="button"
              className={`ck-spaces-rail-item${s.id === selected ? ' is-active' : ''}`}
              aria-current={s.id === selected ? 'page' : undefined}
              onClick={() => onSelect(s.id)}
            >
              <span className="ck-spaces-rail-name">{s.name}</span>
              {s.unread > 0 && (
                <span className="ck-spaces-badge" aria-label={`${s.unread} unread`}>
                  {s.unread}
                </span>
              )}
              <span className="ck-spaces-rail-sub">{s.online > 0 ? `${s.online} online` : 'No one online'}</span>
            </button>
          </li>
        ))}
      </ul>
      <div className="ck-spaces-actions">
        <button type="button" className="ck-spaces-btn" onClick={onNew}>
          New space
        </button>
        <button type="button" className="ck-spaces-btn" onClick={onJoin}>
          Join a space
        </button>
      </div>
    </nav>
  );
}
