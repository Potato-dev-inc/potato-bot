// Skills of a Potato Bot (the bot panel's Skills tab): the written ways of working its owner taught it. Desktop only: the
// Tauri commands `potato_skill_*` in src-tauri/src/commands/potato_skills.rs. The bot makes a skill (when its owner asks);
// the page lists, reads, edits and deletes. The name is the file, so it cannot be changed here.
// `fakeSkillsTransport` is the in-memory twin for tests. Nothing here draws.
import { invoke } from '@tauri-apps/api/core';

export interface Skill {
  /** The file name without `.md`; the key every call uses. */
  slug: string;
  name: string;
  description: string;
  body: string;
}

export interface SkillsTransport {
  list(botId: string): Promise<Skill[]>;
  save(botId: string, slug: string, description: string, body: string): Promise<Skill>;
  /** Moves the skill to the removed folder. The page asks first. */
  remove(botId: string, slug: string): Promise<Skill>;
}

export const tauriSkillsTransport: SkillsTransport = {
  list: (botId) => invoke<Skill[]>('potato_skill_list', { botId }),
  save: (botId, slug, description, body) => invoke<Skill>('potato_skill_save', { botId, slug, description, body }),
  remove: (botId, slug) => invoke<Skill>('potato_skill_delete', { botId, slug }),
};

/** The same limits as the engine (`potato/skills.rs`). */
export const SKILL_LIMITS = { description: 200, file: 4096 } as const;

export const SKILLS_COPY = {
  title: 'Skills',
  intro: 'Ways of working that this bot learned from you. It reads one only when you ask for something that fits.',
  empty: 'No skills yet. Ask the bot to learn one: "Learn how I like my weekly report."',
  loading: 'Loading…',
  loadFailed: 'Could not load the skills.',
  saveFailed: 'Could not save the skill.',
  deleteFailed: 'Could not delete the skill.',
  edit: 'Edit',
  save: 'Save',
  cancel: 'Cancel',
  delete: 'Delete',
  confirm: (name: string) => `Delete "${name}"? The file moves to a removed folder; the bot stops using it.`,
  confirmYes: 'Yes, delete',
  description: 'Description',
  steps: 'Steps',
  noDescription: 'No description',
  saved: (name: string) => `Saved "${name}".`,
  deleted: (name: string) => `Deleted "${name}".`,
  tooLong: (n: number) => `${n} of ${SKILL_LIMITS.file} characters at most.`,
} as const;

/** How many characters the file would have: the same layout the engine writes. */
export function skillSize(name: string, description: string, body: string): number {
  return `# ${name}\nDescription: ${description.trim()}\n\n${body.trim()}\n`.length;
}

// ---- the fake -----------------------------------------------------------------------------------

export interface FakeSkillsTransport extends SkillsTransport {
  readonly calls: { method: keyof SkillsTransport; args: unknown[] }[];
}

export function fakeSkillsTransport(seed: Record<string, Skill[]> = {}, opts: { fail?: string } = {}): FakeSkillsTransport {
  const byBot = new Map(Object.entries(seed).map(([bot, list]) => [bot, list.map((s) => ({ ...s }))]));
  const calls: FakeSkillsTransport['calls'] = [];
  const run = <T>(method: keyof SkillsTransport, args: unknown[], body: () => T): Promise<T> => {
    calls.push({ method, args });
    if (opts.fail) return Promise.reject(new Error(opts.fail));
    try {
      return Promise.resolve(body());
    } catch (error) {
      return Promise.reject(error);
    }
  };
  const skills = (botId: string) => {
    const list = byBot.get(botId) ?? [];
    byBot.set(botId, list);
    return list;
  };
  return {
    calls,
    list: (botId) => run('list', [botId], () => skills(botId).map((s) => ({ ...s }))),
    save: (botId, slug, description, body) =>
      run('save', [botId, slug, description, body], () => {
        const found = skills(botId).find((s) => s.slug === slug);
        if (!found) throw new Error('POTATO_SKILL_NOT_FOUND: That skill is gone.');
        if (!body.trim()) throw new Error('POTATO_SKILL_INVALID: A skill needs some steps. Nothing was saved.');
        found.description = description.trim();
        found.body = body.trim();
        return { ...found };
      }),
    remove: (botId, slug) =>
      run('remove', [botId, slug], () => {
        const list = skills(botId);
        const at = list.findIndex((s) => s.slug === slug);
        if (at < 0) throw new Error('POTATO_SKILL_NOT_FOUND: That skill is gone.');
        return { ...list.splice(at, 1)[0] };
      }),
  };
}
