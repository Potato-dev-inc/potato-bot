import { describe, expect, it } from 'vitest';
import { fakeRoomsTransport, isRoomGone, ROOM_ERROR_TEXT, roomErrorText, roomProblem } from './potato-rooms';

describe('roomProblem', () => {
  it('empty name', () => expect(roomProblem('  ', ['a', 'b'])).toBe('Give the group a name.'));
  it('one bot', () => expect(roomProblem('x', ['a'])).toBe('Pick at least 2 bots.'));
  it('eleven bots', () => expect(roomProblem('x', 'abcdefghijk'.split(''))).toBe('A group can have up to 10 bots.'));
  it('ten bots is fine', () => expect(roomProblem('x', 'abcdefghij'.split(''))).toBeNull());
  it('duplicate', () => expect(roomProblem('x', ['a', 'a'])).toBe('Each bot can be in a group once.'));
  it('ok', () => expect(roomProblem('Weekend', ['a', 'b'])).toBeNull());
});

describe('fake rooms transport', () => {
  it('send adds a reply from a bot one hop deeper', async () => {
    const t = fakeRoomsTransport();
    const room = await t.create('Weekend', ['a', 'b']);
    await t.send(room.id, 'hi');
    await new Promise((r) => setTimeout(r, 0));
    const lines = await t.lines(room.id, 10);
    expect(lines).toHaveLength(2);
    expect(lines[1].from.kind).toBe('bot');
    expect(lines[1].hop).toBe(1);
  });

  it('refuses an empty send with the engine sentence', async () => {
    const t = fakeRoomsTransport();
    const room = await t.create('Weekend', ['a', 'b']);
    await expect(t.send(room.id, '  ')).rejects.toThrow('Write a message first.');
  });
});

describe('roomErrorText', () => {
  const codes = ['POTATO_GROUPS_OFF', 'POTATO_ROOM_NAME', 'POTATO_ROOM_BOTS', 'POTATO_ROOM_LIMIT', 'POTATO_ROOM_GONE', 'POTATO_ROOM_ID', 'POTATO_ROOM_IO'];

  it('has a plain sentence for each code, with or without the app sentence, and never shows the code', () => {
    for (const code of codes) {
      const bare = roomErrorText(new Error(code), 'fallback');
      expect(bare, code).toBe(ROOM_ERROR_TEXT[code]);
      expect(bare).not.toMatch(/POTATO_/);
      expect(roomErrorText(new Error(`${code}: `), 'fallback'), code).toBe(ROOM_ERROR_TEXT[code]);
      expect(roomErrorText(`${code}: Something plain.`, 'fallback'), code).toBe('Something plain.');
    }
  });

  it('matches the sentences the app sends for the codes it sends', () => {
    expect(roomErrorText(new Error('POTATO_ROOM_NAME: Give the group a name.'), 'x')).toBe('Give the group a name.');
    expect(roomErrorText(new Error('POTATO_ROOM_LIMIT: You can have up to 50 groups.'), 'x')).toBe(ROOM_ERROR_TEXT.POTATO_ROOM_LIMIT);
    expect(roomErrorText(new Error('POTATO_ROOM_GONE: This group no longer exists.'), 'x')).toBe(ROOM_ERROR_TEXT.POTATO_ROOM_GONE);
    expect(roomErrorText(new Error('POTATO_ROOM_IO: Could not save the group. Try again.'), 'x')).toBe(ROOM_ERROR_TEXT.POTATO_ROOM_IO);
  });

  it('uses the fallback for an unknown code with no sentence, and knows a gone group', () => {
    expect(roomErrorText(new Error('SOMETHING_ELSE'), 'fallback')).toBe('fallback');
    expect(roomErrorText(undefined, 'fallback')).toBe('fallback');
    expect(isRoomGone(new Error('POTATO_ROOM_GONE: This group no longer exists.'))).toBe(true);
    expect(isRoomGone(new Error('POTATO_ROOM_IO'))).toBe(false);
  });
});
