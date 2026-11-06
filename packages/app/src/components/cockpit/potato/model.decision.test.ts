// The thread rows of decision cards: a card is a `decision` row; its answer row folds into it and is never a row.
import { describe, expect, it } from 'vitest';
import { threadRows } from './model';
import { msg } from './test-helpers';

const card = (id: string) => msg({ id: `c-${id}`, text: 'npm publish', kind: 'decision_card', requestId: id, card: { kind: 'approval', why: 'Ships to npm.', choices: [] } });
const answer = (id: string, value: string) =>
  msg({ id: `r-${id}-${value}`, text: 'Answered.', from: { kind: 'note' }, kind: 'decision_result', requestId: id, card: { kind: 'approval', why: '', choices: [], answer: value } });

describe('threadRows: decision cards', () => {
  it('a card with no answer is an open decision row', () => {
    const rows = threadRows([card('a')]);
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({ type: 'decision', result: null });
  });

  it('the answer folds into its card and is not a row of its own', () => {
    const rows = threadRows([card('a'), answer('a', 'allow_once')]);
    expect(rows).toHaveLength(1);
    expect(rows[0].type).toBe('decision');
    expect(rows[0].type === 'decision' && rows[0].result?.card?.answer).toBe('allow_once');
  });

  it('a cancelled card keeps its cancelled answer', () => {
    const rows = threadRows([card('a'), answer('a', 'cancelled')]);
    expect(rows[0].type === 'decision' && rows[0].result?.card?.answer).toBe('cancelled');
  });

  it('each card finds its own answer', () => {
    const rows = threadRows([card('a'), card('b'), answer('b', 'deny')]);
    expect(rows.map((r) => (r.type === 'decision' ? r.result?.card?.answer ?? null : r.type))).toEqual([null, 'deny']);
  });

  it('an answer with no card is not shown at all', () => {
    expect(threadRows([answer('zzz', 'deny')])).toEqual([]);
  });
});
