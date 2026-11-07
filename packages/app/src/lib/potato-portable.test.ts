// Reading what a person pasted from ChatGPT or Claude, and the steps that make a new bot first and fill it second.
import { describe, expect, it, vi } from 'vitest';
import { fakeCompanionTransport } from './companion';
import { fakePotatoTransport } from './potato';
import { companionName, duplicateBot, fakePortableTransport, makeBotFromDrafts, makeEmptyBot, parseChatExport, restoreAccount } from './potato-portable';

describe('parseChatExport', () => {
  it('reads the Instructions and Memories sections the prompt asks for', () => {
    const out = parseChatExport('Instructions:\nBe brief.\nNo emoji.\n\nMemories:\n- Likes tea\n* Has a cat\n3. Lives in Taipei\n');
    expect(out.instructions).toBe('Be brief.\nNo emoji.');
    expect(out.memories).toEqual(['Likes tea', 'Has a cat', 'Lives in Taipei']);
  });

  it('takes plain pasted text as instructions', () => {
    expect(parseChatExport('Always answer in Traditional Chinese.')).toEqual({ instructions: 'Always answer in Traditional Chinese.', memories: [] });
  });

  it('finds instructions and memories in a ChatGPT or Claude style export', () => {
    const chatgpt = { user_settings: { about_user_message: 'I am a nurse.', about_model_message: 'Be concise.' }, memories: ['Likes tea', { content: 'Has a cat' }] };
    const out = parseChatExport(JSON.stringify(chatgpt));
    expect(out.instructions).toBe('I am a nurse.\n\nBe concise.');
    expect(out.memories).toEqual(['Likes tea', 'Has a cat']);
    const claude = [{ name: 'Project', prompt_template: 'You are my editor.' }];
    expect(parseChatExport(JSON.stringify(claude)).instructions).toBe('You are my editor.');
  });

  it('is bounded: a huge or deeply nested file gives drafts, not a hang or a crash', () => {
    const deep = `${'{"a":'.repeat(5000)}{"memories":["deep"]}${'}'.repeat(5000)}`;
    expect(() => parseChatExport(deep)).not.toThrow();
    const many = { memories: Array.from({ length: 5000 }, (_, i) => `memory ${i}`) };
    expect(parseChatExport(JSON.stringify(many)).memories).toHaveLength(200);
    expect(parseChatExport('{ not json').instructions).toBe('{ not json');
    const long = parseChatExport(JSON.stringify({ memories: ['x'.repeat(5000)] }));
    expect(long.memories[0]).toHaveLength(600);
  });

  it('drops repeated memories', () => {
    expect(parseChatExport(JSON.stringify({ memories: ['A', 'a', 'B'] })).memories).toEqual(['A', 'B']);
  });
});

describe('making a bot first and filling it second', () => {
  const setup = () => {
    const companions = fakeCompanionTransport();
    const potato = fakePotatoTransport({ bots: [] });
    const portable = fakePortableTransport();
    return { companions, potato, portable, create: vi.spyOn(companions, 'create') };
  };

  it('cuts a name to what a companion accepts', () => {
    expect(companionName('  A very long bot name that goes past twenty-four letters  ').length).toBeLessThanOrEqual(24);
    expect(companionName('   ')).toBe('Imported bot');
  });

  it('refuses plainly when no agent is ready', async () => {
    const companions = fakeCompanionTransport({ agents: [{ id: 'x', label: 'X', ok: false, reason: 'no' }] });
    await expect(makeEmptyBot({ companions, potato: fakePotatoTransport({ bots: [] }) }, { name: 'A' })).rejects.toThrow('No agent is ready yet');
  });

  it('a duplicate is a new bot named "<name> copy" filled from the first', async () => {
    const t = setup();
    const id = await duplicateBot(t, { id: 'nimbus', name: 'Nimbus', avatar: 'tofu', persona: 'calm' });
    expect(t.create.mock.calls[0][0]).toMatchObject({ name: 'Nimbus copy', avatar: 'tofu', persona: 'calm' });
    expect(t.portable.calls.find((c) => c.method === 'copyInto')?.args).toEqual(['nimbus', id]);
  });

  it('drafts are saved to the new bot, not typed into any other', async () => {
    const t = setup();
    const id = await makeBotFromDrafts(t, 'From Claude', 'Be brief.', ['Likes tea']);
    expect(t.portable.calls.find((c) => c.method === 'saveDrafts')?.args).toEqual([id, 'Be brief.', ['Likes tea']]);
  });

  it('a restore goes on after a bot that fails, and stops at the room there is', async () => {
    const t = setup();
    const bot = (id: string) => ({ id, bot: { name: id, avatar: 'bit', persona: 'plain', talkPersona: 'fish' }, skills: 0, routines: 0, notes: 0, messages: 0 });
    const preview = { exportedMs: 0, bots: [bot('a'), bot('b'), bot('c')], notes: [], promise: '' };
    const failing = { ...t.portable, accountRestoreBot: vi.fn().mockRejectedValueOnce(new Error('broken')).mockResolvedValue({ instructions: true, skills: 0, routines: 0, notes: 0, chat: 0 }) };
    const out = await restoreAccount({ ...t, portable: failing }, 'D:\\b', preview, 2);
    expect(out.failed).toHaveLength(1);
    expect(out.failed[0]).toContain('broken');
    expect(out.made).toHaveLength(1);
    expect(t.create).toHaveBeenCalledTimes(2);
  });
});
