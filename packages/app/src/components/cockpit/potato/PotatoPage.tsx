// The Potato Bot page in the cockpit's main area (spec section 4): three regions. The bot list on the left, the open
// thread in the middle and, when the person opens it, the bot panel on the right (Computer, Routines, Advanced).
// It is reachable only while the Labs switch is on (the caller renders it only then). A bot is a companion, so
// "New bot" opens the companion sheet and turns what it makes into a bot.
import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { AvatarId, CompanionTransport, CompanionView } from '../../../lib/companion';
import { plainError, type PotatoState, type PotatoTransport } from '../../../lib/potato';
import { isSearchShortcut } from '../../../lib/potato-list';
import { groupCanShow, loadFor, SEARCH_COPY, tauriSearchTransport, type SearchHit, type SearchTransport } from '../../../lib/potato-search';
import type { PaletteItem } from '../../../lib/potato-palette';
import { ROOM_COPY, tauriRoomsTransport, type RoomsTransport } from '../../../lib/potato-rooms';
import { tauriFilesTransport, type FilesTransport } from '../../../lib/potato-files';
import type { SandboxTransport } from '../../../lib/potato-sandbox';
import type { ScreenTransport } from '../../../lib/potatoScreen';
import { usePotatoThread } from '../../../hooks/usePotato';
import { tauriBotSettingsTransport, type BotSettingsTransport } from '../../../lib/potato-botsettings';
import { tauriRoutinesTransport, type RoutinesTransport } from '../../../lib/potato-routines';
import { CompanionSetupSheet } from '../companion/CompanionSetupSheet';
import { DEFAULT_TAB, type PanelTab } from '../../../lib/potato-panel';
import { BotList } from './BotList';
import { CommandPalette } from './CommandPalette';
import { PotatoLayout } from './PotatoLayout';
import { SandboxWizardHost } from './SandboxWizardHost';
import { MoveBotsSheet } from './MoveBotsSheet';
import { StarterPicker } from './StarterPicker';
import { ThreadView } from './ThreadView';
import { pickOpenBot } from './model';
import { usePotatoKeys } from './potato-keys';
import { usePotatoLayout } from './usePotatoLayout';
import { usePotatoRooms } from './usePotatoRooms';

// The group chat and its sheet load on demand, so the main Vite chunk stays under its size limit.
const RoomThread = lazy(() => import('./RoomThread').then((m) => ({ default: m.RoomThread })));
const NewGroupSheet = lazy(() => import('./NewGroupSheet').then((m) => ({ default: m.NewGroupSheet })));
// The Artifacts page and the bot panel (with its Advanced, Computer and Routines tabs) load when opened.
const ArtifactsPage = lazy(() => import('./ArtifactsPage').then((m) => ({ default: m.ArtifactsPage })));
const SearchChatsPage = lazy(() => import('./SearchChatsPage').then((m) => ({ default: m.SearchChatsPage })));
const BotPanel = lazy(() => import('./BotPanel').then((m) => ({ default: m.BotPanel })));
// The quiet fallback while a lazy group piece loads: an empty monochrome region, no spinner.
const LAZY_FALLBACK = <div className="ck-potato-blank" aria-hidden="true" />;

export interface PotatoPageProps {
  state: PotatoState;
  /** Goes up on every change; the open thread reads again when it moves. */
  version: number;
  transport: PotatoTransport;
  companionTransport: CompanionTransport;
  /** The companions: a bot's face and agent come from its companion. Null: no companion backend. */
  companions: CompanionView[] | null;
  openBotId: string | null;
  onOpenBot: (botId: string) => void;
  /** Called after a command, so the state is read again at once. */
  onChanged: () => void;
  /** Called after the sheet made a companion, so the companion list is read again. */
  onCompanionsChanged: () => void;
  onNotify: (text: string) => void;
  onOpenSettings: () => void;
  /** Opens a companion's own page ("Through my agents"). */
  onOpenCompanion?: (id: string) => void;
  /** Opens a helper's pane (the Open button of a finished helper), and says whether the pane is still there. */
  onOpenPane?: (paneId: string) => void;
  paneExists?: (paneId: string) => boolean;
  /** The Advanced tab's calls to the engine. Absent: the app's own. */
  botSettings?: BotSettingsTransport;
  /** The Computer panel's way to the app (the sandbox screen). Absent: no panel. */
  screenTransport?: ScreenTransport;
  /** The sandbox's commands for the Computer tab's upkeep section and its set up card. Absent: neither is drawn. */
  sandboxTransport?: SandboxTransport;
  /** The Computer tab's Files section. Tests pass a fake; the app passes nothing. */
  filesTransport?: FilesTransport;
  /** The routines calls for a starter bot's routine. Absent: the app's own. */
  routines?: RoutinesTransport;
  /** Every running agent pane, for binding one started with the full New agent flow. */
  panes?: readonly { id: string; agentType: string; title?: string | null; lane?: string | null }[];
  /** Opens the New agent page. */
  onOpenNewAgent?: () => void;
  /** The cockpit (ACP permission cards and Stop in the bot's chat). */
  cockpit?: import('../../../hooks/useCockpit').CockpitApi;
  /** The Labs switch `potato_groups_enabled` (with Potato Bot on). Off: no Groups block and no New group sheet. */
  groupsOn?: boolean;
  /** The Labs switch `potato_lend_app_enabled` on Windows. Off: no "Apps from your PC" card on the Computer tab. */
  lendOn?: boolean;
  /** The groups calls. Absent: the app's own. */
  roomsTransport?: RoomsTransport;
  /** The Search chats page's call to the app. Absent: the app's own. */
  searchTransport?: SearchTransport;
}

export function PotatoPage(props: PotatoPageProps) {
  const { state, version, transport, companionTransport, companions, openBotId, onOpenBot, onChanged, onCompanionsChanged, onNotify, onOpenSettings, onOpenCompanion, onOpenPane, paneExists, screenTransport, sandboxTransport, filesTransport = tauriFilesTransport, botSettings, routines, panes, onOpenNewAgent, cockpit, groupsOn = false, lendOn = false, roomsTransport = tauriRoomsTransport, searchTransport = tauriSearchTransport } = props;
  const [sheetOpen, setSheetOpen] = useState(false);
  // Bot groups: the group whose chat is open, and the New group sheet.
  const [openRoomId, setOpenRoomId] = useState<string | null>(null);
  const [groupSheetOpen, setGroupSheetOpen] = useState(false);
  const rooms = usePotatoRooms(roomsTransport, groupsOn);
  const openRoom = groupsOn && openRoomId ? (rooms.rooms.find((r) => r.id === openRoomId) ?? null) : null;
  // The open group was deleted somewhere else (or the switch went off): close it, and say so once.
  const closeRoom = useCallback(() => setOpenRoomId(null), []);
  const roomGone = useCallback(() => {
    setOpenRoomId(null);
    onNotify(ROOM_COPY.gone);
  }, [onNotify]);
  // A group made a moment ago is not in the list until it is read again, so "gone" counts only once it has been seen there.
  const seenRoom = useRef<string | null>(null);
  useEffect(() => {
    if (!openRoomId) {
      seenRoom.current = null;
      return;
    }
    if (!groupsOn) {
      setOpenRoomId(null);
      return;
    }
    if (rooms.rooms.some((r) => r.id === openRoomId)) seenRoom.current = openRoomId;
    else if (rooms.loaded && seenRoom.current === openRoomId) roomGone();
  }, [openRoomId, groupsOn, rooms.loaded, rooms.rooms, roomGone]);
  // "New bot" opens the starter picker (F-2); "Set up by hand" in it opens the sheet. The computer's wizard opens over it.
  const [pickerOpen, setPickerOpen] = useState(false);
  const [moveOpen, setMoveOpen] = useState(false);
  // Ctrl/Cmd+K: the jump palette (bots, groups and pages). The sidebar search box stays for typing a name.
  const [paletteOpen, setPaletteOpen] = useState(false);
  // The Search chats page takes the thread's place while it is open. A result opens its thread and jumps to the message.
  const [searchOpen, setSearchOpen] = useState(false);
  const [roomJump, setRoomJump] = useState<{ id: string; n: number } | null>(null);
  // How many messages a private thread reads: more than usual when a search result is further back than that.
  const [readBack, setReadBack] = useState<{ botId: string; limit: number } | null>(null);
  const openSearch = useCallback(() => {
    setArtifactsOpen(false);
    setSearchOpen(true);
  }, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing || !isSearchShortcut(e)) return;
      e.preventDefault();
      setPaletteOpen((open) => !open);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  const paletteItems = useMemo<PaletteItem[]>(
    () => [
      ...state.bots.map((b) => ({
        id: `bot:${b.id}`,
        label: b.name,
        hint: 'Bot',
        run: () => {
          setArtifactsOpen(false);
          setSearchOpen(false);
          setOpenRoomId(null);
          onOpenBot(b.id);
        },
      })),
      ...(groupsOn
        ? rooms.rooms.map((r) => ({
            id: `room:${r.id}`,
            label: r.name,
            hint: 'Group',
            run: () => {
              setArtifactsOpen(false);
              setSearchOpen(false);
              setOpenRoomId(r.id);
            },
          }))
        : []),
      { id: 'page:search', label: SEARCH_COPY.title, hint: 'Page', run: () => openSearch() },
      { id: 'page:settings', label: 'Settings', hint: 'Page', run: () => onOpenSettings() },
      { id: 'page:new-bot', label: 'New bot', hint: 'Page', run: () => setPickerOpen(true) },
      ...(groupsOn ? [{ id: 'page:new-group', label: 'New group', hint: 'Page', run: () => setGroupSheetOpen(true) }] : []),
    ],
    [state.bots, rooms.rooms, groupsOn, onOpenBot, onOpenSettings],
  );
  const [wizardOpen, setWizardOpen] = useState(false);
  // The Artifacts page takes the thread's place while it is open; opening a bot gives the thread back.
  const [artifactsOpen, setArtifactsOpen] = useState(false);
  // A routine run's "Show reply": the message the thread scrolls to (`n` makes the same message jump again).
  const [jump, setJump] = useState<{ id: string; n: number } | null>(null);
  const open = pickOpenBot(state.bots, openBotId);
  const layout = usePotatoLayout();
  // The tab each bot's panel was last on (not remembered between runs).
  const [tabs, setTabs] = useState<Record<string, PanelTab>>({});
  const tab: PanelTab = (open && tabs[open.id]) || DEFAULT_TAB;
  const setTab = useCallback((id: string, next: PanelTab) => setTabs((was) => ({ ...was, [id]: next })), []);
  usePotatoKeys({ toggleList: layout.toggleList, togglePanel: layout.togglePanel });
  const { messages } = usePotatoThread(transport, open?.id ?? null, version, open?.unread ?? 0, readBack && readBack.botId === open?.id ? readBack.limit : undefined);
  const openHit = (hit: SearchHit) => {
    setSearchOpen(false);
    setArtifactsOpen(false);
    if (hit.kind === 'bot') {
      setOpenRoomId(null);
      setReadBack({ botId: hit.ownerId, limit: loadFor(hit) });
      setJump((was) => ({ id: hit.messageId, n: (was?.n ?? 0) + 1 }));
      onOpenBot(hit.ownerId);
      return;
    }
    setOpenRoomId(hit.ownerId);
    setRoomJump((was) => ({ id: hit.messageId, n: (was?.n ?? 0) + 1 }));
    if (!groupCanShow(hit)) onNotify(SEARCH_COPY.tooOld);
  };

  const byId = useMemo(() => new Map((companions ?? []).map((c) => [c.id, c])), [companions]);
  const avatars = useMemo(() => new Map<string, AvatarId>([...byId].map(([id, c]) => [id, c.avatar])), [byId]);
  const candidates = useMemo(() => (companions ?? []).filter((c) => !state.bots.some((b) => b.id === c.id)), [companions, state.bots]);

  const makeBot = async (companionId: string) => {
    try {
      await transport.addBot(companionId);
      onChanged();
      onOpenBot(companionId);
    } catch (error) {
      onNotify(plainError(error, 'Could not make a bot.'));
    }
  };

  // A candidate that is not wanted: remove it for good, so it stops counting toward the three.
  const deleteCompanion = async (companionId: string) => {
    try {
      await companionTransport.delete(companionId);
      onCompanionsChanged();
      onNotify('Companion deleted.');
    } catch (error) {
      onNotify(plainError(error, 'Could not delete the companion.'));
    }
  };

  // A bot's own brain decides (the engine reports `agents` for it, default or not), not the Settings brain alone.
  const through = !!open && open.brainState === 'agents';
  const panelOpen = layout.panelOpen && !!open && !openRoom;

  // The row menu's commands: each one runs, then the state is read again at once.
  const botCommand = (what: string, run: (botId: string) => Promise<void>) => (botId: string) => {
    run(botId)
      .catch((error) => onNotify(plainError(error, `Could not ${what}.`)))
      .finally(onChanged);
  };
  const botActions = {
    pause: botCommand('pause the bot', (id) => transport.stopBot(id)),
    resume: botCommand('start the bot', (id) => transport.startBot(id)),
    markRead: botCommand('mark the bot read', (id) => transport.markRead(id)),
    archive: botCommand('archive the chat', (id) => transport.setArchived(id, true)),
    restore: botCommand('restore the chat', (id) => transport.setArchived(id, false)),
  };

  const list = (
    <BotList
      bots={state.bots}
      avatars={avatars}
      openId={openRoom ? null : (open?.id ?? null)}
      onOpen={(botId) => {
        setArtifactsOpen(false);
        setSearchOpen(false);
        setOpenRoomId(null);
        onOpenBot(botId);
      }}
      groupsOn={groupsOn}
      rooms={rooms.rooms}
      openRoomId={openRoom?.id ?? null}
      onOpenRoom={(roomId) => {
        setArtifactsOpen(false);
        setSearchOpen(false);
        setOpenRoomId(roomId);
      }}
      onNewGroup={() => setGroupSheetOpen(true)}
      onNew={() => setPickerOpen(true)}
      candidates={candidates}
      onUseCompanion={(id) => void makeBot(id)}
      onDeleteCompanion={(id) => void deleteCompanion(id)}
      mini={layout.listMini}
      onToggleMini={layout.toggleList}
      actions={botActions}
      onArtifacts={() => {
        setSearchOpen(false);
        setArtifactsOpen((was) => !was);
      }}
      artifactsOpen={artifactsOpen}
    />
  );
  const chat = open ? (
    <ThreadView
      key={open.id}
      bot={open}
      messages={messages}
      settings={state.settings}
      avatar={avatars.get(open.id) ?? 'mochi'}
      agentLabel={byId.get(open.id)?.agentLabel ?? 'its agent'}
      transport={transport}
      onOpenSettings={onOpenSettings}
      onNotify={onNotify}
      onChanged={onChanged}
      companionPaneId={byId.get(open.id)?.paneId ?? null}
      panes={panes}
      cockpit={cockpit}
      onOpenCompanion={onOpenCompanion}
      onOpenPane={onOpenPane}
      paneExists={paneExists}
      jumpTo={jump}
      screen={screenTransport}
      onNewBot={() => setPickerOpen(true)}
      panel={{
        open: panelOpen,
        computerShown: panelOpen && tab === 'computer',
        onToggle: layout.togglePanel,
        onOpenComputer: () => {
          setTab(open.id, 'computer');
          layout.setPanelOpen(true);
        },
      }}
    />
  ) : (
    <div className="ck-potato-blank" aria-hidden="true" />
  );
  // A group's chat takes the thread's place. It has no bot panel.
  const groupPanel = openRoom ? (
    <Suspense fallback={LAZY_FALLBACK}>
      <RoomThread
        key={openRoom.id}
        room={openRoom}
        transport={roomsTransport}
        potato={transport}
        bots={state.bots}
        avatars={avatars}
        jumpTo={roomJump}
        onDeleted={() => {
          closeRoom();
          rooms.refresh();
        }}
        onGone={roomGone}
        onNotify={onNotify}
        onChanged={onChanged}
      />
    </Suspense>
  ) : null;
  const thread = searchOpen ? (
    <Suspense fallback={LAZY_FALLBACK}>
      <SearchChatsPage transport={searchTransport} onOpenHit={openHit} onClose={() => setSearchOpen(false)} />
    </Suspense>
  ) : artifactsOpen ? (
    <Suspense fallback={LAZY_FALLBACK}>
      <ArtifactsPage bots={state.bots.map((b) => ({ id: b.id, name: b.name }))} transport={transport.artifacts} onClose={() => setArtifactsOpen(false)} onNotify={onNotify} />
    </Suspense>
  ) : (
    (groupPanel ?? chat)
  );
  const panel = panelOpen && open ? (
    <Suspense fallback={LAZY_FALLBACK}>
      <BotPanel
        key={open.id}
        bot={open}
        avatar={avatars.get(open.id) ?? 'mochi'}
        tab={tab}
        onTab={(next) => setTab(open.id, next)}
        onClose={() => layout.setPanelOpen(false)}
        screen={screenTransport}
        sandbox={sandboxTransport}
        files={screenTransport ? filesTransport : undefined}
        lendOn={lendOn}
        onOpenSettings={onOpenSettings}
        through={through}
        onNotify={onNotify}
        defaultBrain={state.settings.brain}
        botSettings={botSettings}
        settings={state.settings}
        transport={transport}
        onChanged={onChanged}
        companion={byId.get(open.id) ?? null}
        companionTransport={companionTransport}
        onOpenPane={onOpenPane}
        routines={routines}
        onOpenMessage={(id) => setJump((was) => ({ id, n: (was?.n ?? 0) + 1 }))}
      />
    </Suspense>
  ) : null;

  return (
    <>
      <PotatoLayout layout={layout} list={list} thread={thread} panel={panel} />
      {pickerOpen && (
        <StarterPicker
          transports={{ companions: companionTransport, potato: transport, settings: botSettings ?? tauriBotSettingsTransport, routines: routines ?? tauriRoutinesTransport }}
          onMade={(botId) => {
            setPickerOpen(false);
            onCompanionsChanged();
            onChanged();
            onOpenBot(botId);
          }}
          onSetUpComputer={sandboxTransport ? () => setWizardOpen(true) : undefined}
          onSetUpByHand={() => {
            setPickerOpen(false);
            setSheetOpen(true);
          }}
          onMove={() => {
            setPickerOpen(false);
            setMoveOpen(true);
          }}
          onClose={() => setPickerOpen(false)}
        />
      )}
      {moveOpen && (
        <MoveBotsSheet
          companions={companionTransport}
          potato={transport}
          botCount={state.bots.length}
          onMade={(botId) => {
            setMoveOpen(false);
            onCompanionsChanged();
            onChanged();
            onOpenBot(botId);
          }}
          onNotify={onNotify}
          onClose={() => setMoveOpen(false)}
        />
      )}
      {wizardOpen && sandboxTransport && <SandboxWizardHost transport={sandboxTransport} onClose={() => setWizardOpen(false)} />}
      {groupsOn && groupSheetOpen && (
        <Suspense fallback={LAZY_FALLBACK}>
          <NewGroupSheet
            bots={state.bots.map((b) => ({ id: b.id, name: b.name }))}
            transport={roomsTransport}
            onCancel={() => setGroupSheetOpen(false)}
            onDone={(room) => {
              setGroupSheetOpen(false);
              rooms.refresh();
              setArtifactsOpen(false);
              setSearchOpen(false);
              setOpenRoomId(room.id);
            }}
          />
        </Suspense>
      )}
      {paletteOpen && <CommandPalette items={paletteItems} onClose={() => setPaletteOpen(false)} />}
      {sheetOpen && (
        <CompanionSetupSheet
          transport={companionTransport}
          onClose={() => setSheetOpen(false)}
          onCreated={(created) => {
            setSheetOpen(false);
            onCompanionsChanged();
            void makeBot(created.id);
          }}
        />
      )}
    </>
  );
}
