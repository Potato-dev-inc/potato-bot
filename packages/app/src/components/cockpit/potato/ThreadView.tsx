// One bot's thread (spec section 4), put together from its parts: the header, the "Needs you" banner, the first-run
// greeting and choice card (a new bot only), the message list, a card when the bot cannot answer, and the message box. Your bubbles are filled, the bot's are outlined. No colour.
// The computer itself lives in the bot panel (BotPanel); the thread only says when the bot needs the person there.
import { useEffect, useState } from 'react';
import type { AvatarId, CompanionTransport, CompanionView } from '../../../lib/companion';
import { useAcpChatState } from '../../../hooks/useAcpChatState';
import { AGENT_CHOICE_EVENT, SANDBOX_NOTICE, acpPaneOf, readAcpPane, readAgentChoice } from '../../../lib/potato-agent-choice';
import { sendPaneMessage } from '../../../lib/paneOutbox';
import type { CockpitApi } from '../../../hooks/useCockpit';
import { AcpChatDock } from '../../chat/AcpChatDock';
import { AcpConfigChips } from '../../chat/AcpConfigChips';
import { COPY, plainError, type PotatoBotView, type PotatoJob, type PotatoJobFiles, type PotatoMessage, type PotatoSettings, type PotatoTransport, type TalkPersona } from '../../../lib/potato';
import { firstRunDone, isNewBot, markFirstRunDone } from '../../../lib/potato-firstrun';
import type { ScreenTransport } from '../../../lib/potatoScreen';
import { SessionChatPanel } from '../../chat/SessionChatPanel';
import { BrainStateCard } from './BrainStateCard';
import { Composer } from './Composer';
import { ComputerBanner, bannerDue } from './ComputerBanner';
import { FirstMessage } from './FirstMessage';
import { JobBar } from './JobBar';
import { JobFiles } from './JobFiles';
import { ThreadHeader } from './ThreadHeader';
import { Transcript } from './Transcript';
import { useReadAloud } from './useReadAloud';
import { useScreenState } from './useScreenState';
import '../../../styles/cockpit-potato-thread.css';

export { MAX_MESSAGE } from '../../../lib/potato-composer';


/** What the thread needs to know about the bot panel next to it. */
export interface ThreadPanel {
  /** The panel is open. */
  open: boolean;
  /** The panel is open on its Computer tab: the banner has nothing to add. */
  computerShown: boolean;
  onToggle: () => void;
  /** The banner's button: open the panel on the Computer tab. */
  onOpenComputer: () => void;
}

export interface ThreadViewProps {
  bot: PotatoBotView;
  messages: PotatoMessage[];
  settings: PotatoSettings;
  avatar: AvatarId;
  /** The label of the agent that does a helper's work (the companion's agent). */
  agentLabel: string;
  transport: PotatoTransport;
  onOpenSettings: () => void;
  /** Said when a command fails and no card is there to say it. */
  onNotify: (text: string) => void;
  /** Called after a command, so the state is read again at once. */
  onChanged: () => void;
  /** The "Through my agents" brain has no thread of its own here: the thread is the companion's own chat, in its pane. */
  companionPaneId?: string | null;
  /** The companion behind this bot ("Through my agents"): the TUI agent it runs on. */
  companion?: CompanionView | null;
  companionTransport?: CompanionTransport;
  onCompanionsChanged?: () => void;
  /** Every running agent pane: the bot can use one started with the full New agent flow. */
  panes?: readonly { id: string; agentType: string; title?: string | null; lane?: string | null }[];
  /** Opens the New agent page (its folder, separate copy, login and lane choices). */
  onOpenNewAgent?: () => void;
  /** The cockpit: an ACP chat's permission cards and Stop live here, as on its own pane. */
  cockpit?: CockpitApi;
  /** The companion's own page (the "Through my agents" line under the header). */
  onOpenCompanion?: (id: string) => void;
  /** Opens a helper's pane (the Open button of a finished helper). Absent: no button. */
  onOpenPane?: (paneId: string) => void;
  /** Whether that pane is still there; the button is shown only then. Default: yes. */
  paneExists?: (paneId: string) => boolean;
  /** The sandbox screen's way to the app: the "Needs you" banner reads it. Absent: no banner. */
  screen?: ScreenTransport;
  /** The bot panel next to the thread. Absent: no panel button and no banner button. */
  panel?: ThreadPanel;
  /** A message to scroll to and mark for a moment (a routine run's "Show reply"). */
  jumpTo?: { id: string; n: number } | null;
  /** `/new` in the message box: start the new-bot flow. Absent: the command is not offered. */
  onNewBot?: () => void;
  /** Passed in so a test can pin the clock. */
  now?: number;
}

export function ThreadView(props: ThreadViewProps) {
  const { bot, messages, settings, avatar, agentLabel, transport, onOpenSettings, onNotify, onChanged, companionPaneId, panes, cockpit, onOpenCompanion, onOpenPane, paneExists, screen, panel, jumpTo, onNewBot, now = Date.now() } = props;
  const [follow, setFollow] = useState(0);
  const [choice, setChoice] = useState<string>(() => readAgentChoice(bot.id));
  const acpAgentId = acpPaneOf(choice);

  const [acpPane, setAcpPane] = useState<string | null>(() => (acpAgentId ? readAcpPane(bot.id, acpAgentId) : null));
  const [told, setTold] = useState(false);
  // The Advanced panel owns the picker: it writes the choice and announces it here.
  useEffect(() => {
    const sync = (event: Event) => {
      const detail = (event as CustomEvent<{ botId?: string }>).detail;
      if (detail?.botId && detail.botId !== bot.id) return;
      const next = readAgentChoice(bot.id);
      setChoice(next);
      const agent = acpPaneOf(next);
      setAcpPane(agent ? readAcpPane(bot.id, agent) : null);
    };
    const onEvent = (event: Event) => sync(event);
    window.addEventListener(AGENT_CHOICE_EVENT, onEvent);
    return () => window.removeEventListener(AGENT_CHOICE_EVENT, onEvent);
  }, [bot.id]);
  // The pane the bot talks in right now: its companion's TUI terminal, or an ACP chat it started in its own folder.
  // A main work-area pane is never borrowed.
  const chatPaneId = acpAgentId ? acpPane : companionPaneId;
  const chatLane = panes?.find((p) => p.id === chatPaneId)?.lane ?? null;
  const chatIsAcp = !!acpAgentId || chatLane === 'acp';
  const acpState = useAcpChatState(chatPaneId ?? '', chatIsAcp);
  // The engine reports `agents` for a bot whose brain is "through my agents", its own or the default.
  const through = bot.brainState === 'agents';
  const screenState = useScreenState(through ? undefined : screen);
  // A new bot (no messages yet) opens with the scripted first message. Once it has been used, the marker keeps it gone.
  const newBot = isNewBot(bot, messages);
  const [firstDone, setFirstDone] = useState(() => firstRunDone(bot.id));
  useEffect(() => {
    if (newBot) return;
    markFirstRunDone(bot.id);
    setFirstDone(true);
  }, [newBot, bot.id]);
  const showFirst = newBot && !firstDone;
  // "Use the computer" is offered only while the sandbox Labs switch is on.
  const computerOn = screenState?.enabled === true;
  useReadAloud(bot.id, messages, settings.voice);
  // The job bar (Professional mode): read again when the bot starts or stops, gets a card, or has new messages.
  const [job, setJob] = useState<PotatoJob | null>(null);
  // The files the job changed with its file tools (Restore for each); read together with the job.
  const [jobFiles, setJobFiles] = useState<PotatoJobFiles | null>(null);
  useEffect(() => {
    if (!transport.job) return;
    let live = true;
    const read = () => {
      transport.job!(bot.id).then(
        (next) => live && setJob(next),
        () => live && setJob(null),
      );
      transport.jobFiles?.(bot.id).then(
        (next) => live && setJobFiles(next),
        () => live && setJobFiles(null),
      );
    };
    read();
    // The app says `potato://changed` when a job starts, moves or stops (for this bot, or null for every bot): read again then.
    const unsub = transport.onChanged?.((changed) => {
      if (live && (changed === null || changed === bot.id)) read();
    });
    return () => {
      live = false;
      void unsub?.then((fn) => fn()).catch(() => {});
    };
  }, [transport, bot.id, bot.typing, bot.openCards, messages.length]);
  const jobRunning = job !== null && (job.state === 'running' || job.state === 'waiting');

  const run = async (call: () => Promise<unknown>, failed: string) => {
    try {
      await call();
      onChanged();
    } catch (error) {
      onNotify(plainError(error, failed));
    }
  };
  // The same path a typed message takes: the thread follows to the bottom, then the bot gets the words.
  const sendAsPerson = (text: string) => {
    setFollow((n) => n + 1);
    return run(() => transport.send(bot.id, text), 'Could not send that.');
  };

  const head = (
    <ThreadHeader
      bot={bot}
      avatar={avatar}
      brain={bot.talkBrain ?? settings.brain}
      through={through}
      onPersona={(next: TalkPersona) => void run(() => transport.setPersona(bot.id, next), 'Could not change the voice.')}
      onStop={() => void run(() => transport.stopBot(bot.id), 'Could not stop it.')}
      onStart={() => void run(() => transport.startBot(bot.id), 'Could not start it.')}
      panel={panel && { open: panel.open, onToggle: panel.onToggle }}
    />
  );

  if (through) {
    return (
      <section className="ck-potato-thread" aria-label={`${bot.name}, through your agents`}>
        {head}
        <p className="ck-sub ck-potato-through-note">{COPY.agentsOnly}</p>
        {chatPaneId ? (
          // The chat pane: a new pane with the same id (a restart) needs a new chat.
          <>
            <div className="ck-potato-through-agent">
              <button
                type="button"
                className="ck-btn ck-btn--plain"
                onClick={() => {
                  void sendPaneMessage(chatPaneId, SANDBOX_NOTICE)
                    .then(() => {
                      setTold(true);
                      onNotify('Sent the computer and Vercel instructions to the agent.');
                    })
                    .catch((cause: unknown) => onNotify(plainError(cause, 'Could not send that to the agent.')));
                }}
              >
                {told ? 'Instructions sent' : 'Tell this agent about the computer and Vercel'}
              </button>
            </div>
            <div className="ck-scroll ck-fade-in ck-potato-chat" key={chatPaneId}>
              <SessionChatPanel
                paneId={chatPaneId}
                reply={{ blockedReason: null }}
                dock={chatIsAcp && cockpit ? <AcpChatDock c={cockpit} paneId={chatPaneId} paneName={bot.name} /> : undefined}
                slashCommands={chatIsAcp ? acpState?.commands : undefined}
                composerTop={chatIsAcp ? <AcpConfigChips paneId={chatPaneId} state={acpState} /> : undefined}
              />
            </div>
          </>
        ) : (
          <div className="ck-potato-through">
            <p className="ck-sub">{bot.name} has no agent chat yet. Pick an agent or start a chat in Advanced &gt; Model.</p>
            {onOpenCompanion && (
              <button type="button" className="ck-btn ck-btn--plain" onClick={() => onOpenCompanion(bot.id)}>
                Open {bot.name}
              </button>
            )}
          </div>
        )}
      </section>
    );
  }

  return (
    <section className="ck-potato-thread" aria-label={`Chat with ${bot.name}`}>
      {head}
      {job && jobRunning && <JobBar job={job} now={now} onStop={() => void run(() => transport.stopBot(bot.id), 'Could not stop it.')} />}
      {job && jobFiles && (jobFiles.files.length > 0 || jobFiles.overflow) && transport.restoreJobFile && (
        <JobFiles botId={bot.id} list={jobFiles} restore={(jobId, fileId) => transport.restoreJobFile!(bot.id, jobId, fileId)} onRestored={onChanged} />
      )}
      {panel && bannerDue(screenState, bot.id, panel.computerShown) && (
        <ComputerBanner botName={bot.name} reason={screenState?.reason ?? null} onOpen={panel.onOpenComputer} />
      )}
      {showFirst && <FirstMessage name={bot.name} persona={bot.talkPersona} computerOn={computerOn} onPick={sendAsPerson} />}
      <Transcript
        bot={bot}
        messages={messages}
        agentLabel={agentLabel}
        transport={transport}
        onChanged={onChanged}
        onOpenPane={onOpenPane}
        paneExists={paneExists}
        followToken={follow}
        jumpTo={jumpTo}
        hideEmpty={showFirst}
        now={now}
        voice={settings.voice}
        onSendText={sendAsPerson}
        onTakeControl={panel?.onOpenComputer}
        onTakeOver={
          screen || panel
            ? async () => {
                // The person has the screen first, so the bot's calls wait; then the computer opens where they can type.
                await screen?.takeOver();
                panel?.onOpenComputer();
              }
            : undefined
        }
      />

      <BrainStateCard
        name={bot.name}
        state={bot.brainState}
        lastError={bot.lastError}
        limitCents={bot.limitCents}
        onStart={async () => {
          await transport.startBot(bot.id);
          onChanged();
        }}
        onOpenSettings={onOpenSettings}
      />

      <Composer
        bot={bot}
        transport={transport}
        onChanged={onChanged}
        onNotify={onNotify}
        onSent={() => setFollow((n) => n + 1)}
        actions={{ onNewBot, onOpenComputer: panel?.onOpenComputer }}
      />
    </section>
  );
}
