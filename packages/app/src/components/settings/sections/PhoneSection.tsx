// Settings > Phone: one row that holds the whole phone panel (see ../PhoneSettings.tsx).
import type { RowDef } from '../model';
import { PhoneSettings } from '../PhoneSettings';

export const phoneRows: RowDef[] = [
  {
    id: 'phone-panel',
    label: 'Phone',
    hint: 'Use your bots from your phone: pair, sign out, and fix a connection.',
    keywords: 'phone pair qr devices tunnel firewall port address',
    render: () => <PhoneSettings />,
  },
];
