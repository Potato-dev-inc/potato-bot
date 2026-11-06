// The right bot panel (spec: Rakazo's side panel): who the bot is, a close button, and four tabs. Computer holds the
// bot's sandbox screen; Routines lists its scheduled prompts. Every tab stays mounted while the
// panel is open, so the screen does not reload and a draft in Advanced survives a tab switch.
import type { AvatarId } from '../../../lib/companion';
import type { CompanionTransport, CompanionView } from '../../../lib/companion';
import type { BrainKind, PotatoSettings, PotatoTransport } from '../../../lib/potato';
import { PANEL_COPY, PANEL_TABS, type PanelTab } from '../../../lib/potato-panel';
import type { RoutinesTransport } from '../../../lib/potato-routines';
import type { FilesTransport } from '../../../lib/potato-files';
import type { SandboxTransport } from '../../../lib/potato-sandbox';
import type { BotSettingsTransport } from '../../../lib/potato-botsettings';
import type { ScreenTransport } from '../../../lib/potatoScreen';
import { Avatar } from '../../companion/Avatar';
import { SegTabs } from '../SegTabs';
import { AdvancedTab } from './AdvancedTab';
import { useBotComputerOff } from './useBotComputerOff';
import { ComputerTab } from './ComputerTab';
import { RoutinesTab } from './RoutinesTab';
import { SkillsTab } from './SkillsTab';
import type { SkillsTransport } from '../../../lib/potato-skills';
import { Glyph } from './ui/Glyph';
import '../../../styles/cockpit-potato-panel.css';

export interface BotPanelProps {
  bot: { id: string; name: string };
  avatar: AvatarId;
  tab: PanelTab;
  onTab: (tab: PanelTab) => void;
  onClose: () => void;
  screen?: ScreenTransport;
  /** The sandbox's commands (start, stop, set up). Absent: the Computer tab has no upkeep section. */
  sandbox?: SandboxTransport;
  /** The Computer tab's Files section. Absent: no Files section. */
  files?: FilesTransport;
  /** Opens Settings > Potato Bot (Sandbox). */
  onOpenSettings?: () => void;
  /** "Through my agents": the bot has no computer of its own. */
  through: boolean;
  onNotify: (text: string) => void;
  /** The Settings brain: what a bot without its own brain uses (the Advanced tab's Model row). */
  defaultBrain?: BrainKind;
  /** After a rename or a delete: the page reads its state again. */
  onChanged?: () => void;
  /** Tests pass a fake; the app passes nothing. */
  botSettings?: BotSettingsTransport;
  /** The Read replies aloud switch: the page's settings and the transport that saves them. */
  settings?: PotatoSettings;
  transport?: PotatoTransport;
  /** The bot's companion: the TUI agent "Through my agents" runs on. */
  companion?: CompanionView | null;
  companionTransport?: CompanionTransport;
  /** Opens a running helper's pane from the Advanced tab's Open work. */
  onOpenPane?: (paneId: string) => void;
  /** The Routines tab's calls. Tests pass a fake; the app passes nothing. */
  routines?: RoutinesTransport;
  /** A routine run's "Show reply": scrolls the thread to that message. */
  onOpenMessage?: (messageId: string) => void;
  /** The Skills tab's calls. Tests pass a fake; the app passes nothing. */
  skills?: SkillsTransport;
}

export function BotPanel({ bot, avatar, tab, onTab, onClose, screen, sandbox, files, onOpenSettings, through, onNotify, defaultBrain, onChanged, botSettings, settings, transport, companion, companionTransport, onOpenPane, routines, onOpenMessage, skills }: BotPanelProps) {
  const computerOff = useBotComputerOff(bot.id, tab === 'computer', botSettings);
  return (
    <div className="ck-potato-botpanel">
      <header className="ck-potato-panel-head">
        <Avatar id={avatar} size={24} state="idle" name={bot.name} />
        <h2 className="ck-potato-panel-title">{bot.name}</h2>
        <button type="button" className="ck-icon-btn" aria-label={PANEL_COPY.close} title={PANEL_COPY.close} onClick={onClose}>
          <Glyph name="close" />
        </button>
      </header>
      <div className="ck-potato-panel-tabs">
        <SegTabs options={PANEL_TABS.map((t) => ({ ...t }))} value={tab} onChange={onTab} label="Bot panel" small />
      </div>
      <div className="ck-scroll ck-potato-panel-body">
        <div className="ck-potato-tabpanel" role="tabpanel" aria-label="Computer" hidden={tab !== 'computer'}>
          <ComputerTab bot={bot} avatar={avatar} screen={screen} sandbox={sandbox} files={files} through={through} onNotify={onNotify} onOpenSettings={onOpenSettings} computerOff={computerOff} onOpenTools={() => onTab('advanced')} />
        </div>
        <div className="ck-potato-tabpanel" role="tabpanel" aria-label="Routines" hidden={tab !== 'routines'}>
          <RoutinesTab bot={bot} transport={routines} onOpenMessage={onOpenMessage} />
        </div>
        <div className="ck-potato-tabpanel" role="tabpanel" aria-label="Skills" hidden={tab !== 'skills'}>
          <SkillsTab bot={bot} transport={skills} onNotify={onNotify} />
        </div>
        <div className="ck-potato-tabpanel" role="tabpanel" aria-label="Advanced" hidden={tab !== 'advanced'}>
          <AdvancedTab bot={bot} defaultBrain={defaultBrain} transport={botSettings} onChanged={onChanged} onNotify={onNotify} settings={settings} potatoTransport={transport} companion={companion} companionTransport={companionTransport} onOpenPane={onOpenPane} />
        </div>
      </div>
    </div>
  );
}
