// The sections of Settings. Potato Bot is the product now; Labs switches stay to turn it and its
// computer off.
import type { LaterItem, SectionDef } from './model';
import { potatoRows } from './sections/PotatoSection';
import { phoneRows } from './sections/PhoneSection';
import { spacesRows } from './sections/SpacesRows';

export const BASE_SECTIONS: SectionDef[] = [
  { id: 'potato', title: 'Potato Bot', blurb: 'Bots, their brains, helpers and their computer.', rows: [...potatoRows, ...spacesRows] },
  { id: 'phone', title: 'Phone', blurb: 'Use your bots from your phone.', rows: phoneRows },
];

export const SECTIONS: SectionDef[] = BASE_SECTIONS;

/** Features that are planned but not built. Shown as plain text on the "Coming later" page. */
export const LATER_ITEMS: LaterItem[] = [];
