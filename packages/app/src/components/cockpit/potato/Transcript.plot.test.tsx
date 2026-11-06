// @vitest-environment jsdom
// The chat with a chart the bot drew (`render_plot`), through the real Transcript: a bar, line or pie is drawn inline as
// an SVG by our own code; a label is text and never markup; an old row shows as plain text.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { fakePotatoTransport, type PotatoChart, type PotatoMessage } from '../../../lib/potato';
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

const chartRow = (chart: PotatoChart): PotatoMessage => ({
  id: 'm-chart',
  at: NOW - 60_000,
  from: { kind: 'bot', name: 'Fish' },
  kind: 'text',
  text: `Chart: ${chart.title}`,
  chart,
});

async function show(messages: PotatoMessage[]) {
  const props: TranscriptProps = {
    bot: bot(),
    messages,
    agentLabel: 'Claude',
    transport: fakePotatoTransport({ bots: [bot()] }),
    onChanged: () => {},
    now: NOW,
    openLink: () => {},
  };
  await act(async () => {
    root.render(<Transcript {...props} />);
  });
}

const pts = [
  { label: 'Mon', value: 3 },
  { label: 'Tue', value: 5 },
  { label: 'Wed', value: 2 },
];

describe('Transcript: a chart the bot drew', () => {
  it('draws a bar chart inline, one bar per point, with the title and the labels', async () => {
    await show([chartRow({ kind: 'bar', title: 'Visits', points: pts })]);
    const figure = host.querySelector('figure.ck-potato-plot');
    expect(figure).not.toBeNull();
    expect(figure?.querySelector('svg')).not.toBeNull();
    expect(figure?.querySelectorAll('rect.ck-potato-plot-bar')).toHaveLength(3);
    expect(figure?.textContent).toContain('Visits');
    expect(figure?.textContent).toContain('Tue');
    expect(figure?.getAttribute('aria-label')).toContain('Visits');
    // The chart replaces the "Chart: ..." text bubble, it does not repeat it.
    expect(host.textContent?.match(/Visits/g)).toHaveLength(1);
  });

  it('draws a line as a path with a dot for each point', async () => {
    await show([chartRow({ kind: 'line', title: 'Trend', points: pts })]);
    expect(host.querySelectorAll('path.ck-potato-plot-line')).toHaveLength(1);
    expect(host.querySelectorAll('circle.ck-potato-plot-dot')).toHaveLength(3);
  });

  it('draws a pie as one slice per non-zero value and a legend with the shares', async () => {
    await show([chartRow({ kind: 'pie', title: 'Split', points: [...pts, { label: 'Zero', value: 0 }] })]);
    expect(host.querySelectorAll('.ck-potato-plot-slice')).toHaveLength(3);
    expect(host.textContent).toContain('Mon');
    expect(host.textContent).toContain('30%');
  });

  it('a label with markup in it is shown as text and creates no element', async () => {
    const evil = '<img src=x onerror=alert(1)><script>1</script>';
    await show([chartRow({ kind: 'bar', title: evil, points: [{ label: evil, value: 1 }] })]);
    expect(host.querySelector('img')).toBeNull();
    expect(host.querySelector('script')).toBeNull();
    expect(host.querySelector('figure')?.textContent).toContain('<img src=x');
  });

  it('shows a flat or negative-only chart without breaking (no NaN in the drawing)', async () => {
    await show([chartRow({ kind: 'bar', title: 'Flat', points: [{ label: 'a', value: 0 }, { label: 'b', value: -4 }] })]);
    await show([chartRow({ kind: 'line', title: 'Same', points: [{ label: 'a', value: 7 }, { label: 'b', value: 7 }] })]);
    expect(host.innerHTML).not.toContain('NaN');
    expect(host.innerHTML).not.toContain('Infinity');
  });

  it('an old text row still shows as plain text with no chart', async () => {
    await show([{ id: 'm-old', at: NOW - 60_000, from: { kind: 'bot', name: 'Fish' }, kind: 'text', text: 'Hello there' }]);
    expect(host.querySelector('.ck-potato-plot')).toBeNull();
    expect(host.textContent).toContain('Hello there');
  });
});
