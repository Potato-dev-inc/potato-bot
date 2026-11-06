import { describe, expect, it } from 'vitest';
import { fakeSearchTransport, groupCanShow, loadFor, searchWords, snippetParts, type SearchHit } from './potato-search';

const hit = (over: Partial<SearchHit> = {}): SearchHit => ({
  kind: 'bot',
  ownerId: 'fish',
  ownerName: 'Fish',
  messageId: 'm-1',
  at: 1_700_000_000_000,
  speaker: 'You',
  snippet: 'plan the Lisbon trip',
  marks: [[9, 15]],
  fromEnd: 0,
  ...over,
});

describe('snippetParts', () => {
  it('cuts the snippet at its marks', () => {
    expect(snippetParts(hit())).toEqual([
      { text: 'plan the ', hit: false },
      { text: 'Lisbon', hit: true },
      { text: ' trip', hit: false },
    ]);
  });
  it('ignores marks that are out of range or overlap, and counts characters not code units', () => {
    expect(snippetParts(hit({ snippet: '旅行 lisbon', marks: [[3, 9], [4, 6], [8, 99]] })).map((p) => p.text)).toEqual(['旅行 ', 'lisbon']);
    expect(snippetParts(hit({ marks: [] }))).toEqual([{ text: 'plan the Lisbon trip', hit: false }]);
  });
});

describe('loadFor and groupCanShow', () => {
  it('reads a private thread back far enough for the hit, never past the store cap', () => {
    expect(loadFor(hit({ fromEnd: 5 }))).toBe(200);
    expect(loadFor(hit({ fromEnd: 250 }))).toBe(280);
    expect(loadFor(hit({ fromEnd: 9000 }))).toBe(5000);
  });
  it('a group chat shows only its newest 500 lines', () => {
    expect(groupCanShow(hit({ kind: 'group', fromEnd: 499 }))).toBe(true);
    expect(groupCanShow(hit({ kind: 'group', fromEnd: 500 }))).toBe(false);
  });
});

describe('searchWords and the fake', () => {
  it('collapses spaces and cuts at 100 characters', () => {
    expect(searchWords('  a   b ')).toBe('a b');
    expect(searchWords('x'.repeat(150))).toHaveLength(100);
  });
  it('the fake needs every word', async () => {
    const t = fakeSearchTransport([hit(), hit({ messageId: 'm-2', snippet: 'only trip' })]);
    expect((await t.search('lisbon TRIP')).hits.map((h) => h.messageId)).toEqual(['m-1']);
    expect(t.calls).toEqual([{ query: 'lisbon TRIP' }]);
  });
});
