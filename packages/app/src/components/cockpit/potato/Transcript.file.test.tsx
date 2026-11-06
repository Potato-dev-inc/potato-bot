// @vitest-environment jsdom
// The chat with a file the bot sent (`send_file`), through the real Transcript: the card shows the name and size with
// Open and Save; an image is drawn in the chat; an old text row still shows as plain text with no card.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { fakePotatoTransport, type PotatoMessage } from '../../../lib/potato';
import { formatSize } from '../../../lib/potato-sandbox';
import { Transcript, type TranscriptProps } from './Transcript';
import { bot } from './test-helpers';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const NOW = new Date(2026, 10, 5, 15, 0).getTime();

const fileRow = (id: string, name: string, kind: 'pdf' | 'image', size: number): PotatoMessage => ({
  id: `m-${id}`,
  at: NOW - 60_000,
  from: { kind: 'bot', name: 'Fish' },
  kind: 'text',
  text: `Sent a file: ${name}`,
  file: { id, name, kind, size },
});

const transport = () =>
  fakePotatoTransport({
    bots: [bot()],
    artifacts: {
      items: [
        { id: 'a-pdf', name: 'report.pdf', bot: bot().id, kind: 'pdf', size: 2048, created: NOW, version: 1, file: 'a-pdf.pdf' },
        { id: 'a-png', name: 'chart.png', bot: bot().id, kind: 'image', size: 900, created: NOW, version: 1, file: 'a-png.png' },
      ],
    },
  });

async function show(messages: PotatoMessage[]) {
  const props: TranscriptProps = {
    bot: bot(),
    messages,
    agentLabel: 'Claude',
    transport: transport(),
    onChanged: () => {},
    now: NOW,
    openLink: () => {},
  };
  await act(async () => {
    root.render(<Transcript {...props} />);
  });
}

describe('Transcript: a file the bot sent', () => {
  it('shows the card with the name, the size, Open and Save', async () => {
    await show([fileRow('a-pdf', 'report.pdf', 'pdf', 2048)]);
    const card = host.querySelector('[aria-label="report.pdf"]');
    expect(card).not.toBeNull();
    expect(card?.textContent).toContain('report.pdf');
    expect(card?.textContent).toContain(formatSize(2048));
    const buttons = Array.from(card?.querySelectorAll('button') ?? []).map((b) => b.textContent);
    expect(buttons).toEqual(['Open', 'Save']);
    // A file row has no "Save as file" in its toolbar: the card's Save is the one.
    expect(host.textContent).not.toContain('Save as file');
  });

  it('draws an image inline in the chat', async () => {
    await show([fileRow('a-png', 'chart.png', 'image', 900)]);
    const picture = host.querySelector('img.ck-potato-file-image') as HTMLImageElement | null;
    expect(picture).not.toBeNull();
    expect(picture?.getAttribute('alt')).toBe('chart.png');
    expect(picture?.getAttribute('src')?.startsWith('data:image/')).toBe(true);
  });

  it('keeps an old text row as plain text, with no card', async () => {
    const old: PotatoMessage = { id: 'm-old', at: NOW - 60_000, from: { kind: 'bot', name: 'Fish' }, kind: 'text', text: 'Hello there' };
    await show([old]);
    expect(host.querySelector('.ck-potato-file')).toBeNull();
    expect(host.textContent).toContain('Hello there');
  });
});
