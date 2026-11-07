// Moving bots in and out: one bot in one file, a copy of a bot, a folder with every bot and chat for a new PC, and drafts
// from ChatGPT or Claude. The engine does the reading, the cleaning and the writing (`potato/service/portable.rs`); this file
// is the calls, the plain words, the ChatGPT/Claude reader, and the steps that make a new bot first and then fill it.
import { invoke } from '@tauri-apps/api/core';
import type { AvatarId, CompanionPersona, CompanionTransport } from './companion';
import { localized, t } from './i18n';
import type { PotatoTransport } from './potato';

export interface PortableBotInfo {
  name: string;
  avatar: string;
  persona: string;
  talkPersona: string;
}
export interface PortableSkill {
  name: string;
  description: string;
  body: string;
}
export interface PortablePreview {
  bot: PortableBotInfo;
  instructions: string;
  skills: PortableSkill[];
  routines: { title: string; prompt: string; summary: string }[];
  memory: { folder: string; name: string; chars: number }[];
  /** What was cut or left out on the way in. */
  notes: string[];
  promise: string;
}
export interface PortableChoices {
  instructions: boolean;
  skills: boolean;
  routines: boolean;
  memory: boolean;
}
export interface PortableApplied {
  instructions: boolean;
  skills: number;
  routines: number;
  notes: number;
  chat: number;
}
export interface AccountBotPreview {
  id: string;
  bot: PortableBotInfo;
  skills: number;
  routines: number;
  notes: number;
  messages: number;
}
export interface AccountPreview {
  exportedMs: number;
  bots: AccountBotPreview[];
  notes: string[];
  promise: string;
}

export interface PortableTransport {
  exportName(botId: string): Promise<string>;
  exportSave(botId: string, memory: boolean, path: string): Promise<string>;
  fileInspect(path: string): Promise<PortablePreview>;
  fileApply(botId: string, path: string, choices: PortableChoices): Promise<PortableApplied>;
  copyInto(fromId: string, toId: string): Promise<PortableApplied>;
  accountExport(parent: string, chats: boolean): Promise<string>;
  accountInspect(dir: string): Promise<AccountPreview>;
  accountRestoreBot(dir: string, id: string, botId: string): Promise<PortableApplied>;
  readText(path: string): Promise<string>;
  saveDrafts(botId: string, instructions: string, memories: string[]): Promise<PortableApplied>;
}

export const tauriPortableTransport: PortableTransport = {
  exportName: (botId) => invoke<string>('potato_bot_export_name', { botId }),
  exportSave: (botId, memory, path) => invoke<string>('potato_bot_export_save', { botId, memory, path }),
  fileInspect: (path) => invoke<PortablePreview>('potato_bot_file_inspect', { path }),
  fileApply: (botId, path, choices) => invoke<PortableApplied>('potato_bot_file_apply', { botId, path, choices }),
  copyInto: (fromId, toId) => invoke<PortableApplied>('potato_bot_copy_into', { fromId, toId }),
  accountExport: (parent, chats) => invoke<string>('potato_account_export', { parent, chats }),
  accountInspect: (dir) => invoke<AccountPreview>('potato_account_inspect', { dir }),
  accountRestoreBot: (dir, id, botId) => invoke<PortableApplied>('potato_account_restore_bot', { dir, id, botId }),
  readText: (path) => invoke<string>('potato_import_read_text', { path }),
  saveDrafts: (botId, instructions, memories) => invoke<PortableApplied>('potato_import_drafts_save', { botId, instructions, memories }),
};

/** A transport for tests: it records every call and answers from what the test gives it. */
export interface FakePortableTransport extends PortableTransport {
  calls: { method: string; args: unknown[] }[];
}
export function fakePortableTransport(seed: Partial<{ preview: PortablePreview; account: AccountPreview; text: string; fail: string }> = {}): FakePortableTransport {
  const calls: FakePortableTransport['calls'] = [];
  const done = (): PortableApplied => ({ instructions: true, skills: 0, routines: 0, notes: 0, chat: 0 });
  const rec = <T>(method: string, args: unknown[], value: () => T): Promise<T> => {
    calls.push({ method, args });
    if (seed.fail) return Promise.reject(new Error(seed.fail));
    return Promise.resolve(value());
  };
  const preview: PortablePreview = seed.preview ?? {
    bot: { name: 'Nimbus', avatar: 'mochi', persona: 'calm', talkPersona: 'fish' },
    instructions: 'Be kind.',
    skills: [],
    routines: [],
    memory: [],
    notes: [],
    promise: PORTABLE_COPY.promiseFallback,
  };
  return {
    calls,
    exportName: (botId) => rec('exportName', [botId], () => 'nimbus.potato-bot.json'),
    exportSave: (botId, memory, path) => rec('exportSave', [botId, memory, path], () => path),
    fileInspect: (path) => rec('fileInspect', [path], () => preview),
    fileApply: (botId, path, choices) => rec('fileApply', [botId, path, choices], done),
    copyInto: (fromId, toId) => rec('copyInto', [fromId, toId], done),
    accountExport: (parent, chats) => rec('accountExport', [parent, chats], () => `${parent}\\potato-bots-20261107-120000`),
    accountInspect: (dir) => rec('accountInspect', [dir], () => seed.account ?? { exportedMs: 0, bots: [], notes: [], promise: PORTABLE_COPY.accountPromise }),
    accountRestoreBot: (dir, id, botId) => rec('accountRestoreBot', [dir, id, botId], done),
    readText: (path) => rec('readText', [path], () => seed.text ?? ''),
    saveDrafts: (botId, instructions, memories) => rec('saveDrafts', [botId, instructions, memories], done),
  };
}

// ---- words ------------------------------------------------------------------------------------------------------------

export const PORTABLE_COPY = localized({
  moveTitle: 'Move bots',
  moveIntro: 'Bring a bot in from a file, from ChatGPT or Claude, or from a backup. Nothing is made until you have seen what is in it.',
  tabFile: 'Bot file',
  tabChat: 'ChatGPT or Claude',
  tabBackup: 'Backup',
  importFile: 'Open a bot file…',
  fileHint: 'A bot file ends in .potato-bot.json. It holds a name, instructions, skills, routines and memory, and no keys.',
  promiseFallback: 'A file adds only text. It cannot turn on tools, connect an account, or change how much this bot asks first, and routines come in paused.',
  accountPromise: 'This copy holds no keys, tokens or sign-ins. You add those again on this PC.',
  contains: 'This file has',
  tickInstructions: 'Instructions',
  tickSkills: 'Skills',
  tickRoutines: 'Routines (they start paused)',
  tickMemory: 'Memory notes',
  makeBot: 'Make this bot',
  making: 'Making…',
  made: (name: string) => t('{name} is ready.', { name }),
  noAgent: 'No agent is ready yet. Set one up first.',
  exportBot: 'Export bot',
  exporting: 'Exporting…',
  exportMemory: 'Include what it remembers',
  exportHint: 'The file holds its name, voice, instructions, skills and routines. It never holds keys, tokens, connector sign-ins or cookies.',
  exported: (path: string) => t('Saved to {path}', { path }),
  duplicate: 'Duplicate bot',
  duplicating: 'Duplicating…',
  duplicated: (name: string) => t('{name} is ready. Its routines are paused.', { name }),
  copySuffix: ' copy',
  chatTitle: 'Bring in from ChatGPT or Claude',
  chatIntro: 'Paste your custom instructions or memories, or open the file you exported. You review everything before it is saved.',
  pasteLabel: 'Paste here',
  pastePlaceholder: 'Paste your custom instructions and memories…',
  openChatFile: 'Open an export file…',
  readIt: 'Read it',
  draftsTitle: 'Review the drafts',
  draftInstructions: 'Instructions',
  draftMemories: 'Memories to keep',
  noDrafts: 'Nothing that looks like instructions or memories was found. Paste the text instead.',
  botName: 'Bot name',
  copyPrompt: 'Copy prompt',
  promptCopied: 'Prompt copied.',
  guideTitle: 'What to copy, and from where',
  backupTitle: 'Back up every bot',
  backupHint: 'Saves a folder with each bot and its chat. It holds no keys, tokens or sign-ins: you add those again on a new PC.',
  backupMakeWithChats: 'Include chats',
  backupMake: 'Back up to a folder…',
  backedUp: (path: string) => t('Backup saved in {path}', { path }),
  restoreTitle: 'Restore from a backup',
  restoreOpen: 'Open a backup folder…',
  restoreAll: 'Restore these bots',
  restoring: 'Restoring…',
  restored: (n: number) => (n === 1 ? t('1 bot restored.') : t('{n} bots restored.', { n })),
  needsRoom: 'You have ten bots, the most there can be. Delete one to make room.',
  close: 'Close',
});

/** What to copy from where, for the two assistants. The menus can be named a little differently in your version. */
export const PROMPT_GUIDE = {
  chatgpt: [
    'Open ChatGPT, then your profile menu, then Settings and Personalization.',
    'Copy what is in Custom instructions: both boxes, one after the other.',
    'Under Memory choose Manage, and copy the memories you want to keep.',
  ],
  claude: [
    'Open Claude, then Settings.',
    'Copy your personal preferences (the box that tells Claude about you and how to answer).',
    'If you use a Project, copy its instructions. If Memory is on, copy what Claude remembers about you.',
  ],
  note: 'You can also ask the old assistant to write it down for you: choose Copy prompt, send it there, and paste the answer here.',
};

/** The prompt to send to the old assistant. The answer is read back by `parseChatExport`. */
export const IMPORT_PROMPT = [
  'I am moving to a new assistant. Please write down, exactly as stored, in this form and nothing else:',
  '',
  'Instructions:',
  '(my custom instructions, word for word)',
  '',
  'Memories:',
  '- one thing you remember about me per line',
  '',
  'Do not include passwords, keys or private numbers.',
].join('\n');

// ---- reading ChatGPT or Claude ----------------------------------------------------------------------------------------

export interface ChatDrafts {
  instructions: string;
  memories: string[];
}

const INSTRUCTION_KEYS = ['custom_instructions', 'about_model_message', 'about_user_message', 'prompt_template', 'system_prompt', 'personal_preferences', 'instructions'];
const MEMORY_KEYS = ['memories', 'memory', 'saved_memories', 'memory_entries'];
const MAX_NODES = 50_000;
const MAX_MEMORIES = 200;
const MAX_MEMORY_CHARS = 600;

function clean(text: string): string {
  return text.replace(/\r\n/g, '\n').trim();
}

function memoryTexts(value: unknown): string[] {
  if (typeof value === 'string') return clean(value).split('\n').map((l) => l.replace(/^\s*[-*•]\s*/, '').trim()).filter(Boolean);
  if (!Array.isArray(value)) return [];
  const out: string[] = [];
  for (const item of value) {
    if (typeof item === 'string') out.push(clean(item));
    else if (item && typeof item === 'object') {
      const o = item as Record<string, unknown>;
      const text = [o.content, o.text, o.memory, o.value].find((v) => typeof v === 'string');
      if (typeof text === 'string') out.push(clean(text));
    }
  }
  return out.filter(Boolean);
}

/** Looks through a parsed export for the places the two assistants keep instructions and memories. Bounded: it visits at most 50 000 values. */
function fromJson(root: unknown): ChatDrafts {
  const instructions: string[] = [];
  const memories: string[] = [];
  const stack: { value: unknown; depth: number }[] = [{ value: root, depth: 0 }];
  let seen = 0;
  while (stack.length > 0 && seen < MAX_NODES) {
    const { value, depth } = stack.pop()!;
    seen += 1;
    if (!value || typeof value !== 'object' || depth > 8) continue;
    if (Array.isArray(value)) {
      for (const v of value) stack.push({ value: v, depth: depth + 1 });
      continue;
    }
    for (const [key, v] of Object.entries(value as Record<string, unknown>)) {
      if (INSTRUCTION_KEYS.includes(key) && typeof v === 'string' && clean(v)) instructions.push(clean(v));
      else if (MEMORY_KEYS.includes(key) && (typeof v === 'string' || Array.isArray(v))) memories.push(...memoryTexts(v));
      else if (v && typeof v === 'object') stack.push({ value: v, depth: depth + 1 });
    }
  }
  return { instructions: [...new Set(instructions)].join('\n\n'), memories };
}

/** Pasted text: "Instructions:" and "Memories:" sections (what the prompt asks for) or, failing that, the whole text as instructions. */
function fromText(text: string): ChatDrafts {
  const body = clean(text);
  const lines = body.split('\n');
  const head = (line: string) => line.trim().replace(/^#+\s*/, '').replace(/[:：]\s*$/, '').toLowerCase();
  const at = (names: string[]) => lines.findIndex((l) => names.includes(head(l)));
  const i = at(['instructions', 'custom instructions']);
  const m = at(['memories', 'memory', 'what you remember about me']);
  if (i === -1 && m === -1) return { instructions: body, memories: [] };
  const section = (from: number, other: number) => {
    if (from === -1) return [];
    const end = other > from ? other : lines.length;
    return lines.slice(from + 1, end);
  };
  const instructions = clean(section(i, m).join('\n'));
  const memories = section(m, i).map((l) => l.replace(/^\s*[-*•\d.)]+\s*/, '').trim()).filter(Boolean);
  return { instructions: instructions.replace(/^\(.*\)$/s, '').trim(), memories };
}

/** The drafts in what a person pasted or in an export file's text. Nothing is saved: the person reviews the drafts first. */
export function parseChatExport(text: string): ChatDrafts {
  const body = text.replace(/^﻿/, '').trim();
  let drafts: ChatDrafts | null = null;
  if (body.startsWith('{') || body.startsWith('[')) {
    try {
      drafts = fromJson(JSON.parse(body));
    } catch {
      drafts = null;
    }
  }
  const result = drafts ?? fromText(body);
  const seen = new Set<string>();
  const memories: string[] = [];
  for (const raw of result.memories) {
    const memory = raw.slice(0, MAX_MEMORY_CHARS).trim();
    if (!memory || seen.has(memory.toLowerCase())) continue;
    seen.add(memory.toLowerCase());
    memories.push(memory);
    if (memories.length >= MAX_MEMORIES) break;
  }
  return { instructions: result.instructions, memories };
}

// ---- making a bot, then filling it ------------------------------------------------------------------------------------

export interface PortableMakers {
  companions: Pick<CompanionTransport, 'agents' | 'create'>;
  potato: Pick<PotatoTransport, 'addBot'>;
  portable: PortableTransport;
}

const AVATARS: AvatarId[] = ['mochi', 'bit', 'nib', 'tofu', 'lumen', 'hoot', 'sprout', 'kit'];
const PERSONAS: CompanionPersona[] = ['cheerful', 'calm', 'coach', 'plain', 'custom'];
const COMPANION_NAME_MAX = 24;

/** A companion name the app accepts: trimmed and cut to 24 letters. */
export function companionName(name: string): string {
  return (name.replace(/\p{C}/gu, ' ').trim().slice(0, COMPANION_NAME_MAX).trim() || 'Imported bot');
}

/** Makes the companion and the bot (the first ready agent runs it, the default folder), and returns the bot's id. */
export async function makeEmptyBot(t: Pick<PortableMakers, 'companions' | 'potato'>, spec: { name: string; avatar?: string; persona?: string }): Promise<string> {
  const agents = await t.companions.agents();
  const agentType = agents.find((a) => a.ok)?.id;
  if (!agentType) throw new Error(PORTABLE_COPY.noAgent);
  const avatar = AVATARS.find((a) => a === spec.avatar) ?? 'bit';
  const persona = PERSONAS.find((p) => p === spec.persona) ?? 'plain';
  const view = await t.companions.create({ name: companionName(spec.name), avatar, persona, agentType, folder: '', autoStart: true, memoryReview: true, napAt: { h: 4, m: 0 }, checkins: [] });
  await t.potato.addBot(view.id);
  return view.id;
}

/** A bot file, after the person ticked what to bring in: makes the bot, then fills it. */
export async function makeBotFromFile(t: PortableMakers, path: string, preview: PortablePreview, choices: PortableChoices, name = preview.bot.name): Promise<string> {
  const botId = await makeEmptyBot(t, { name, avatar: preview.bot.avatar, persona: preview.bot.persona });
  await t.portable.fileApply(botId, path, choices);
  return botId;
}

/** "Duplicate bot": a second bot with the first one's instructions, skills, memory and routines (paused). */
export async function duplicateBot(t: PortableMakers, from: { id: string; name: string; avatar?: string; persona?: string }): Promise<string> {
  const copy = `${from.name.slice(0, COMPANION_NAME_MAX - PORTABLE_COPY.copySuffix.length).trim()}${PORTABLE_COPY.copySuffix}`;
  const botId = await makeEmptyBot(t, { name: copy, avatar: from.avatar, persona: from.persona });
  await t.portable.copyInto(from.id, botId);
  return botId;
}

/** Drafts the person reviewed: a new bot with those instructions and memories. */
export async function makeBotFromDrafts(t: PortableMakers, name: string, instructions: string, memories: string[]): Promise<string> {
  const botId = await makeEmptyBot(t, { name });
  await t.portable.saveDrafts(botId, instructions, memories);
  return botId;
}

/** Every bot of a backup folder, one after the other. A bot that fails is reported and the rest go on. Stops at the bot limit. */
export async function restoreAccount(t: PortableMakers, dir: string, preview: AccountPreview, room: number): Promise<{ made: string[]; failed: string[] }> {
  const made: string[] = [];
  const failed: string[] = [];
  for (const entry of preview.bots.slice(0, Math.max(0, room))) {
    try {
      const botId = await makeEmptyBot(t, { name: entry.bot.name, avatar: entry.bot.avatar, persona: entry.bot.persona });
      await t.portable.accountRestoreBot(dir, entry.id, botId);
      made.push(botId);
    } catch (e) {
      failed.push(`${entry.bot.name}: ${e instanceof Error ? e.message : String(e)}`);
    }
  }
  return { made, failed };
}
