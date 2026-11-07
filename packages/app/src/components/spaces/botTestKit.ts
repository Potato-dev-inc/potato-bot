// Shared helpers for the bot screen tests: a bot list fixture and React-friendly typing.
import { act } from 'react';
import type { SpaceBotList, SpaceBotRow } from '../../lib/spaces';

export const botRow = (over: Partial<SpaceBotRow> = {}): SpaceBotRow => ({
  id: 'b1', name: 'Ren的fish', ownerName: 'Ren', ownerMember: 'mem-ren', mine: true, muted: false, silenced: false,
  mode: 'called', memory: true, dayCap: 50, potato: 'p1', usedToday: 3, heldBackToday: 2, ...over,
});
export const otherRow = (over: Partial<SpaceBotRow> = {}): SpaceBotRow =>
  botRow({ id: 'b2', name: 'Sam的cat', ownerName: 'Sam', ownerMember: 'mem-sam', mine: false, silenced: null, mode: null,
    memory: null, dayCap: null, potato: null, usedToday: 0, heldBackToday: 0, ...over });
export const botList = (over: Partial<SpaceBotList> = {}): SpaceBotList => ({
  space: 's1', isOwner: true, rules: { perPerson: 3, channels: null }, mineCount: 1, addBlocked: null, pendingAdd: [],
  bots: [botRow(), otherRow()], ...over,
});

/** Types into a controlled React input. */
export function typeInto(el: HTMLInputElement, value: string): void {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  act(() => {
    setter?.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
}
