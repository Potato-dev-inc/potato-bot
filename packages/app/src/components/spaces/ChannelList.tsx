import type { SpaceChannel } from '../../lib/spaces';

export interface ChannelListProps {
  channels: SpaceChannel[];
  selected: string | null;
  isOwner: boolean;
  onSelect: (id: string) => void;
  onNew: () => void;
}

/** The channel column: active channels with unread counts. The owner gets `New channel`. */
export function ChannelList({ channels, selected, isOwner, onSelect, onNew }: ChannelListProps) {
  const active = channels.filter((c) => !c.archived);
  const full = active.length >= 20;
  return (
    <nav className="ck-spaces-channels" aria-label="Channels">
      <ul>
        {active.map((c) => (
          <li key={c.id}>
            <button
              type="button"
              className={`ck-spaces-channel${c.id === selected ? ' is-active' : ''}${c.unread > 0 ? ' is-unread' : ''}`}
              aria-current={c.id === selected ? 'page' : undefined}
              onClick={() => onSelect(c.id)}
            >
              <span># {c.name}</span>
              {c.unread > 0 && (
                <span className="ck-spaces-badge" aria-label={`${c.unread} unread`}>
                  {c.unread}
                </span>
              )}
            </button>
          </li>
        ))}
      </ul>
      {isOwner && (
        <button
          type="button"
          className="ck-spaces-quiet"
          disabled={full}
          title={full ? 'This space has 20 channels. Archive one first.' : undefined}
          onClick={onNew}
        >
          New channel
        </button>
      )}
    </nav>
  );
}
