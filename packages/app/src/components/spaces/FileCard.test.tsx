// @vitest-environment jsdom
// The file card: Get file, progress, Save to Downloads, the safety line, and no Open button.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SpaceFile } from '../../lib/spaces';
import { FileCard, formatSize } from './FileCard';

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

const file = (over: Partial<SpaceFile> = {}): SpaceFile => ({
  sha: 'ab', name: 'notes.txt', size: 2_516_582, have: false, state: 'idle', percent: 0, ...over,
});

function mount(over: Partial<Parameters<typeof FileCard>[0]> = {}) {
  const props = { file: file(), error: null as string | null, savedAs: null as string | null, onGet: vi.fn(), onSave: vi.fn(), ...over };
  act(() => root.render(<FileCard {...props} />));
  return props;
}
const buttons = () => [...host.querySelectorAll('button')];
const text = () => host.textContent ?? '';

describe('FileCard', () => {
  it('not here shows Get file', () => {
    const p = mount();
    expect(text()).toContain('notes.txt · 2.4 MB');
    const b = buttons().find((x) => x.textContent === 'Get file')!;
    act(() => b.click());
    expect(p.onGet).toHaveBeenCalledTimes(1);
  });

  it('getting shows percent', () => {
    mount({ file: file({ state: 'getting', percent: 40 }) });
    expect(text()).toContain('Getting file… 40%');
    expect(buttons()).toHaveLength(0);
  });

  it('here shows Save to Downloads', () => {
    const p = mount({ file: file({ have: true }) });
    const b = buttons().find((x) => x.textContent === 'Save to Downloads')!;
    act(() => b.click());
    expect(p.onSave).toHaveBeenCalledTimes(1);
  });

  it('saved line', () => {
    mount({ file: file({ have: true }), savedAs: 'a (1).txt' });
    expect(text()).toContain('Saved to Downloads as a (1).txt.');
  });

  it('error shows as an alert', () => {
    mount({ error: 'No one online has this file right now. Try again later.' });
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('No one online has this file right now. Try again later.');
  });

  it('safety line always shows', () => {
    for (const f of [file(), file({ have: true }), file({ state: 'getting', percent: 3 })]) {
      mount({ file: f });
      expect(text()).toContain('Files from other people can be harmful. Only open files you expect.');
    }
  });

  it('formatSize', () => {
    expect(formatSize(512)).toBe('512 B');
    expect(formatSize(2048)).toBe('2 KB');
    expect(formatSize(2_516_582)).toBe('2.4 MB');
  });

  it('no open button exists', () => {
    for (const f of [file(), file({ have: true })]) {
      mount({ file: f });
      expect(buttons().some((b) => /^open/i.test(b.textContent ?? ''))).toBe(false);
    }
  });
});
