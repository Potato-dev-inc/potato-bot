// The words of the Computer tab and the full-window computer (P3). Plain English, short sentences.
import { STATE_WORD, formatSize, osLabel, type SandboxState, type SandboxView } from './potato-sandbox';

export const COMPUTER_COPY = {
  screenOf: (name: string) => `${name}’s screen`,
  takeControl: 'Take control',
  /** The person hands the computer back. */
  imDone: "I'm done",
  skip: 'Skip',
  /** The hover pill on the preview, and its label. */
  open: 'Open',
  openLabel: 'Open in full window',
  inFullWindow: 'Open in full window',
  retry: 'Retry',
  asleep: 'Asleep',
  wake: 'Wake',
  waking: 'Waking up…',
  couldNotWake: 'Could not start',
  wakeFirst: 'Wake the computer first.',
  // the full window
  overlayTitle: (name: string) => `${name}’s computer`,
  close: 'Close',
  confirmTitle: (name: string) => `Hand the computer back to ${name}?`,
  confirmBody: (name: string) => `${name} goes on from where you stopped.`,
  keepControl: 'Keep control',
  // the clipboard buttons (control view only)
  paste: 'Paste',
  pasteHint: 'Paste what you copied on this PC into the computer. Text only.',
  copyOut: 'Copy',
  copyHint: 'Copy what the computer copied onto this PC. Text only.',
  pasted: 'Pasted.',
  copied: 'Copied.',
  clipboardEmpty: 'Your clipboard has no text.',
  clipboardBlocked: 'Windows would not let the app use the clipboard. Click in the screen and press Ctrl+V to paste.',
  clipboardTooBig: 'That is too long to paste here.',
  handBack: 'Hand back',
  // the maintenance section
  sectionTitle: 'The computer',
  start: 'Start',
  stop: 'Stop',
  restart: 'Restart',
  settings: 'Open sandbox settings',
  setUpTitle: 'Set up the computer',
  setUpBody: (name: string) => `${name} can use a small private computer, with its own browser and files. It cannot see your files or accounts.`,
  setUpButton: 'Set up the computer',
  settingUp: (percent: number) => `Setting up the computer… ${Math.round(percent)}%`,
  showSetUp: 'Show',
  unfinishedTitle: "Setup didn't finish",
  unfinishedBody: 'Setting up the computer was cut off, for example because the app was closed. You can pick it up where it stopped, or start over.',
  unfinishedBodyNoResume: 'Setting up the computer was cut off, and it cannot be picked up from here. Start over to set it up again.',
  continueSetUp: 'Continue setup',
  continuing: 'Continuing…',
  startOver: 'Start over',
  startOverTitle: 'Start over?',
  startOverBody: 'The unfinished computer is deleted and set up begins again from the start.',
  labsOff: (name: string) => `${name} has no computer here. Turn on Sandbox in Settings > General > Labs.`,
};

/** `Running`, `Running. Uses 3.7 GB`. */
export function sandboxLine(view: SandboxView): string {
  const parts = [STATE_WORD[view.state]];
  if (view.activeDesktop && view.desktops.length > 0) parts.push(osLabel(view.activeDesktop));
  if (view.diskBytes !== null && (view.state === 'running' || view.state === 'stopped')) parts.push(`Uses ${formatSize(view.diskBytes)}`);
  return parts.join('. ');
}

/** Whether the screen can be looked at: the sandbox runs, or nobody can say (no sandbox reading). */
export function screenReady(state: SandboxState | null): boolean {
  return state === null || state === 'running';
}

/** The "Apps from your PC" card and its picker (Labs `potato_lend_app_enabled`, Windows only). */
export const LEND_COPY = {
  title: 'Apps from your PC',
  empty: (name: string) => `${name} can use Windows apps you already have open, like LINE.`,
  emptyHint: 'You can take it back at any time. It never moves your mouse.',
  lendButton: 'Lend an open app',
  stateFree: (name: string) => `Ready. ${name} can use it now.`,
  picturePaused: 'Picture paused',
  stateBot: (name: string) => `${name} is using it`,
  stateYou: (name: string) => `You have control. ${name} is waiting for you.`,
  stateNeedsYou: (name: string, reason: string | null) => (reason ? `${name} needs you: ${reason}` : `${name} needs you`),
  takeControl: 'Take control',
  handBack: 'Hand back',
  bringBack: 'Bring back',
  /** Shown in place of the row for a few seconds after Bring back. */
  returned: (app: string) => `${app} is yours again.`,
  modeLabel: 'Show the app',
  visible: 'Show it',
  visibleHint: 'Stays where it is, with a border',
  hidden: 'Keep it out of my way',
  hiddenHint: 'Still running, see-through',
  pickerTitle: 'Lend an app',
  pickerIntro: 'Pick an app you have open on this PC.',
  /** `app` is the picked app's name, or null before a pick. */
  willUse: (name: string, app: string | null) =>
    `${name} will read and type in ${app ?? 'this app'} as you. It won't type passwords. Take control any time.`,
  elevated: 'Opened as administrator. Close it and open it normally to lend it.',
  showElevated: (count: number) => `Show ${count} more that can't be lent`,
  otherApps: 'Other apps',
  noWindows: 'No open apps yet. Open the app now and it will show up here.',
  /** `app` is the picked app's name, or null before a pick. */
  lend: (app: string | null) => (app ? `Lend ${app}` : 'Lend this app'),
  cancel: 'Cancel',
  loading: 'Looking for open apps…',
};
