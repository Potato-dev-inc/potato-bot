// The bot list on the left of the Potato Bot page (spec section 4). Top: the Bots | Activity switch, a search box and the
// button that makes the list a strip. Bots: "Needs you" pinned on top, the rest by recent; each row has its face, name,
// time, last message, a status line and the mood word. Activity: recent things across the bots. The row menu (Pause or
// Resume, Mark read) appears only when the page gives the list its actions. "New bot" opens the companion sheet; a
// companion that is not a bot yet can be turned into one with one click. Its width comes from the page (the
// `--potato-list-w` variable); in the strip mode (`mini`) only the faces are left, with a dot for a bot that needs you.
import { useEffect, useMemo, useRef, useState } from 'react';
import type { AvatarId, CompanionView } from '../../../lib/companion';
import { COPY, type PotatoBotView } from '../../../lib/potato';
import { activityEvents, botMenuItems, listSections, LIST_COPY, type BotMenuAction } from '../../../lib/potato-list';
import { ARTIFACT_COPY } from '../../../lib/potato-artifacts';
import { ROOM_COPY, type RoomView } from '../../../lib/potato-rooms';
import { IconPlus } from '../Icons';
import { ActivityList } from './ActivityList';
import { BotContextMenu } from './BotContextMenu';
import { BotListRow } from './BotListRow';
import { BotListSearch } from './BotListSearch';
import { GroupRow } from './GroupRow';
import { Glyph } from './ui/Glyph';
import '../../../styles/cockpit-potato-list.css';

/** What the row menu can do. Only the ones the page passes appear in the menu. */
export interface BotListActions {
  pause?: (botId: string) => void;
  resume?: (botId: string) => void;
  markRead?: (botId: string) => void;
  archive?: (botId: string) => void;
  restore?: (botId: string) => void;
}

export interface BotListProps {
  bots: PotatoBotView[];
  /** The face of each bot (it is a companion). A bot without a face gets the first one. */
  avatars: ReadonlyMap<string, AvatarId>;
  openId: string | null;
  onOpen: (botId: string) => void;
  onNew: () => void;
  /** Companions that are not bots yet. */
  candidates: CompanionView[];
  onUseCompanion: (companionId: string) => void;
  /** Removes a candidate for good, so it stops counting toward the three-companion limit. */
  onDeleteCompanion?: (companionId: string) => void;
  /** The strip: faces only. */
  mini?: boolean;
  /** Makes the list a strip, or opens it again. Absent: no button. */
  onToggleMini?: () => void;
  /** The row menu's actions. Absent (or empty): no "..." button and no right-click menu. */
  actions?: BotListActions;
  /** Opens the Artifacts page (its button sits in the list's header). Absent: no button. */
  onArtifacts?: () => void;
  artifactsOpen?: boolean;
  /** Bot groups (Labs). Off: no Groups block at all. */
  groupsOn?: boolean;
  rooms?: RoomView[];
  /** The group whose thread is open, if any (the page sets it; a bot's row is not highlighted then). */
  openRoomId?: string | null;
  onOpenRoom?: (roomId: string) => void;
  onNewGroup?: () => void;
  /** Passed in so a test can pin the clock. */
  now?: number;
}

type View = 'bots' | 'activity';
const VIEWS: { id: View; label: string }[] = [
  { id: 'bots', label: LIST_COPY.bots },
  { id: 'activity', label: LIST_COPY.activity },
];
const ALL_ACTIONS: readonly BotMenuAction[] = ['pause', 'resume', 'markRead', 'archive', 'restore'];

export function BotList({ bots: allBots, avatars, openId, onOpen, onNew, candidates, onUseCompanion, onDeleteCompanion, mini = false, onToggleMini, actions, onArtifacts, artifactsOpen = false, groupsOn = false, rooms = [], openRoomId = null, onOpenRoom, onNewGroup, now = Date.now() }: BotListProps) {
  const [view, setView] = useState<View>('bots');
  const [archivedOpen, setArchivedOpen] = useState(false);
  // Archived chats leave the list and sit under Archived; the strip never shows them.
  const bots = useMemo(() => allBots.filter((b) => !b.archived), [allBots]);
  const archivedBots = useMemo(() => allBots.filter((b) => b.archived), [allBots]);
  const [query, setQuery] = useState('');
  const [menu, setMenu] = useState<{ botId: string; x: number; y: number } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const available = ALL_ACTIONS.filter((action) => typeof actions?.[action] === 'function');
  const menuOn = available.length > 0;
  // The strip never filters: it shows every bot, the ones that need you first.
  const sections = useMemo(() => listSections(bots, mini ? '' : query), [bots, query, mini]);
  const events = useMemo(() => (view === 'activity' && !mini ? activityEvents(bots, query) : []), [bots, query, view, mini]);
  const menuBot = menu ? allBots.find((b) => b.id === menu.botId) : undefined;
  const menuItems = menuBot ? botMenuItems(menuBot, available) : [];

  const renderRow = (bot: PotatoBotView) => (
    <li key={bot.id}>
      <BotListRow
        bot={bot}
        avatar={avatars.get(bot.id) ?? 'mochi'}
        active={bot.id === openId}
        mini={mini}
        now={now}
        menu={menuOn}
        menuOpen={menu?.botId === bot.id}
        onOpen={() => onOpen(bot.id)}
        onMenu={(x, y) => setMenu({ botId: bot.id, x, y })}
      />
    </li>
  );

  const section = (title: string | undefined, rows: PotatoBotView[]) => (
    <section className="ck-potato-section" key={title ?? 'all'}>
      {title && <h2 className="ck-potato-section-title">{title}</h2>}
      <ul className="ck-potato-group" aria-label={title ?? LIST_COPY.bots}>
        {rows.map(renderRow)}
      </ul>
    </section>
  );

  const noRows = mini ? sections.needs.length + sections.rest.length === 0 : view === 'bots' && sections.needs.length + sections.rest.length === 0;

  return (
    <nav className={`ck-potato-list${mini ? ' is-mini' : ''}`} aria-label="Conversations">
      {(!mini || onToggleMini) && (
        <div className="ck-potato-list-head">
          {!mini && (
            <div className="ck-seg ck-seg--sm ck-potato-views" role="group" aria-label={LIST_COPY.viewLabel}>
              {VIEWS.map((option) => (
                <button key={option.id} type="button" className="ck-seg-btn" aria-pressed={view === option.id} onClick={() => setView(option.id)}>
                  {option.label}
                </button>
              ))}
            </div>
          )}
          {onArtifacts && (
            <button
              type="button"
              className={`ck-icon-btn ck-potato-artifacts-open${artifactsOpen ? ' is-on' : ''}`}
              aria-label={ARTIFACT_COPY.title}
              aria-pressed={artifactsOpen}
              title={ARTIFACT_COPY.title}
              onClick={onArtifacts}
            >
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                <path d="M5.5 4.5L2.5 8l3 3.5M10.5 4.5l3 3.5-3 3.5M9 3l-2 10" />
              </svg>
            </button>
          )}
          {onToggleMini && (
            <button
              type="button"
              className="ck-icon-btn ck-potato-list-toggle"
              aria-label={mini ? LIST_COPY.show : LIST_COPY.hide}
              title={`${mini ? LIST_COPY.show : LIST_COPY.hide} (Ctrl+B)`}
              onClick={onToggleMini}
            >
              <Glyph name="list" />
            </button>
          )}
        </div>
      )}
      {!mini && <BotListSearch value={query} onChange={setQuery} inputRef={inputRef} />}
      <div className="ck-potato-list-scroll">
        {bots.length === 0 && !mini && <p className="ck-sub ck-potato-empty">{COPY.emptyList}</p>}
        {bots.length > 0 && noRows && view === 'bots' && <p className="ck-sub ck-potato-empty">{LIST_COPY.noMatch(query.trim())}</p>}
        {mini && <ul className="ck-potato-group">{[...sections.needs, ...sections.rest].map(renderRow)}</ul>}
        {!mini && view === 'bots' && (
          <>
            {groupsOn && (
              <section className="ck-potato-section" aria-label={ROOM_COPY.listHeader}>
                <div className="ck-potato-section-head">
                  <h2 className="ck-potato-section-title">{ROOM_COPY.listHeader}</h2>
                  <button type="button" className="ck-btn ck-btn--plain ck-btn--sm ck-potato-newgroup-open" onClick={onNewGroup}>
                    {ROOM_COPY.newGroup}
                  </button>
                </div>
                <ul className="ck-potato-group" aria-label={ROOM_COPY.listHeader}>
                  {rooms.map((room) => (
                    <li key={room.id}>
                      <GroupRow room={room} active={room.id === openRoomId} onOpen={() => onOpenRoom?.(room.id)} />
                    </li>
                  ))}
                </ul>
              </section>
            )}
            {groupsOn && <h2 className="ck-potato-section-title ck-potato-bots-title">{ROOM_COPY.botsHeader}</h2>}
            {sections.needs.length > 0 && section(LIST_COPY.needsYou, sections.needs)}
            {sections.rest.length > 0 && section(sections.needs.length > 0 ? LIST_COPY.recent : undefined, sections.rest)}
          </>
        )}
        {!mini && view === 'activity' && <ActivityList events={events} now={now} onOpen={onOpen} />}
      </div>
      {!mini && archivedBots.length > 0 && (
        <section className="ck-potato-section ck-potato-archived" aria-label={LIST_COPY.archivedTitle(archivedBots.length)}>
          <div className="ck-potato-section-head">
            <button type="button" className="ck-btn ck-btn--plain ck-btn--sm ck-potato-archived-toggle" aria-expanded={archivedOpen} onClick={() => setArchivedOpen((open) => !open)}>
              {LIST_COPY.archivedTitle(archivedBots.length)}
            </button>
          </div>
          {archivedOpen && <ul className="ck-potato-group">{archivedBots.map(renderRow)}</ul>}
        </section>
      )}
      {menu && menuBot && menuItems.length > 0 && (
        <BotContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems}
          onPick={(action) => {
            setMenu(null);
            actions?.[action]?.(menuBot.id);
          }}
          onClose={() => setMenu(null)}
        />
      )}
      <div className="ck-potato-list-foot">
        {candidates.map((companion) => (
          <div key={companion.id} className="ck-potato-make-row">
            <button
              type="button"
              className="ck-row ck-potato-make"
              aria-label={mini ? `Make ${companion.name} a bot` : undefined}
              title={mini ? `Make ${companion.name} a bot` : undefined}
              onClick={() => onUseCompanion(companion.id)}
            >
              <span className="ck-row-glyph">
                <IconPlus />
              </span>
              {!mini && (
                <span className="ck-row-main">
                  <span className="ck-row-title">Make {companion.name} a bot</span>
                </span>
              )}
            </button>
            {!mini && onDeleteCompanion && (
              <button
                type="button"
                className="ck-icon-btn ck-potato-make-delete"
                aria-label={`Delete ${companion.name}`}
                title={`Delete ${companion.name}`}
                onClick={() => onDeleteCompanion(companion.id)}
              >
                <Glyph name="close" size={14} />
              </button>
            )}
          </div>
        ))}
        <button type="button" className="ck-row ck-potato-make" aria-label={mini ? 'New bot' : undefined} title={mini ? 'New bot' : undefined} onClick={onNew}>
          <span className="ck-row-glyph">
            <IconPlus />
          </span>
          {!mini && (
            <span className="ck-row-main">
              <span className="ck-row-title">New bot</span>
            </span>
          )}
        </button>
      </div>
    </nav>
  );
}
