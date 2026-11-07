// Spaces (Labs `spaces_enabled`): the people column. Presence comes only from the space data (online, away, offline).
import type { ReactNode } from 'react';
import type { PresenceState, SpaceDetailBot, SpaceMember } from '../../lib/spaces';

const LABEL: Record<PresenceState, string> = { online: 'Online', away: 'Away', offline: 'Offline' };

export function PresenceDot({ state }: { state: PresenceState }) {
  return <span className={`ck-spaces-dot is-${state}`} role="img" aria-label={LABEL[state]} title={LABEL[state]} />;
}

const ORDER: Record<PresenceState, number> = { online: 0, away: 1, offline: 2 };

/** Owner first, then online before away before offline, then by name. */
export function sortMembers(list: SpaceMember[]): SpaceMember[] {
  return [...list].sort((a, b) => {
    const owner = (a.role === 'owner' ? 0 : 1) - (b.role === 'owner' ? 0 : 1);
    if (owner !== 0) return owner;
    const presence = ORDER[a.presence] - ORDER[b.presence];
    if (presence !== 0) return presence;
    const an = a.name.toLowerCase();
    const bn = b.name.toLowerCase();
    return an < bn ? -1 : an > bn ? 1 : 0;
  });
}

export interface MemberListProps {
  members: SpaceMember[];
  /** Narrow screens: the People button in the header sets this. */
  open?: boolean;
  /** Later phases pass the per-member menu (Remove, Block). Until then nothing. */
  menu?: (m: SpaceMember) => ReactNode;
  /** The bots of this space. They are listed under their own heading, never counted as people. */
  bots?: SpaceDetailBot[];
}

export function MemberList({ members, open = false, menu, bots = [] }: MemberListProps) {
  const sorted = sortMembers(members);
  return (
    <aside className={`ck-spaces-members ck-spaces-people${open ? ' is-open' : ''}`} aria-label="People">
      <h2 className="ck-spaces-feed-title">People ({members.length})</h2>
      <ul>
        {sorted.map((m) => (
          <li key={m.member} className={`ck-spaces-member${m.blocked ? ' is-blocked' : ''}`}>
            <PresenceDot state={m.presence} />
            <span className="ck-spaces-member-name">
              {m.name}
              {m.isMe ? ' (you)' : ''}
            </span>
            {m.role === 'owner' && <span className="ck-spaces-role">Owner</span>}
            {m.blocked && <span className="ck-spaces-role">Blocked</span>}
            {menu?.(m)}
          </li>
        ))}
      </ul>
      {bots.length > 0 && (
        <>
          <h2 className="ck-spaces-feed-title ck-spaces-subhead">Bots</h2>
          <ul aria-label="Bots">
            {bots.map((b) => (
              <li key={b.id} className={`ck-spaces-botmember${b.muted ? ' is-muted' : ''}`}>
                <span className="ck-spaces-botmark" aria-hidden="true" />
                <span className="ck-spaces-member-name">{b.name}</span>
                <span className="ck-spaces-role">Bot · {b.ownerName}{b.mine ? ' (yours)' : ''}{b.muted ? ' · Hidden' : ''}</span>
              </li>
            ))}
          </ul>
        </>
      )}
    </aside>
  );
}
