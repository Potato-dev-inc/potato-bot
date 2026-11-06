// One group in the bot list (plan 12, slice G9): up to three stacked initials of its bots, the name, how many bots,
// and a dot for unread lines. It uses the bot rows' classes so it looks like them. Opening it is the page's job (G10
// builds the thread).
import type { RoomView } from '../../../lib/potato-rooms';
import { ROOM_COPY } from '../../../lib/potato-rooms';

export interface GroupRowProps {
  room: RoomView;
  active: boolean;
  onOpen: () => void;
}

export function GroupRow({ room, active, onOpen }: GroupRowProps) {
  const initials = room.bots.slice(0, 3).map((bot) => bot.name.trim().charAt(0).toUpperCase() || '?');
  const classes = ['ck-row', 'ck-potato-conv', 'ck-potato-grouprow', active && 'is-active', room.unread > 0 && 'is-unread'];
  return (
    <button type="button" className={classes.filter(Boolean).join(' ')} aria-pressed={active} onClick={onOpen} data-room-id={room.id}>
      <span className="ck-row-glyph ck-potato-grouprow-faces" aria-hidden="true">
        {initials.map((letter, i) => (
          <span key={`${i}-${letter}`} className="ck-potato-grouprow-face">
            {letter}
          </span>
        ))}
      </span>
      <span className="ck-potato-conv-main">
        <span className="ck-potato-conv-top">
          <span className="ck-row-title">{room.name}</span>
        </span>
        <span className="ck-potato-conv-bottom">
          <span className="ck-potato-last">{ROOM_COPY.botsCount(room.bots.length)}</span>
          {room.unread > 0 && <span className="ck-potato-dot" role="img" aria-label={`${room.unread} unread`} />}
        </span>
      </span>
    </button>
  );
}
