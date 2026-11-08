// @vitest-environment jsdom
// The service starts on its own task after the Labs switch goes on, so the first `spaces_view` can fail ("can't go online").
// Nothing pushes an event until a command has worked once, so the hook has to ask again by itself.
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const calls = vi.hoisted(() => ({ n: 0 }));
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => {
    calls.n += 1;
    if (calls.n < 3) throw "Spaces can't go online right now. Check your internet connection.";
    return { spaces: [], requests: [], joining: [], invites: [], online: true, relayed: false, showOnline: true };
  }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

import { useSpaces, VIEW_RETRY_MS, type SpacesApi } from '../../lib/spaces';

let api: SpacesApi;
function Probe() {
  api = useSpaces(true);
  return null;
}

beforeEach(() => { calls.n = 0; vi.useFakeTimers(); });
afterEach(() => { vi.useRealTimers(); });

describe('useSpaces', () => {
  it('asks again after a failed view until the service answers, then clears the error', async () => {
    const host = document.createElement('div');
    const root = createRoot(host);
    await act(async () => root.render(<Probe />));
    expect(api.view).toBeNull();
    expect(api.error).toContain("can't go online");
    await act(async () => { await vi.advanceTimersByTimeAsync(VIEW_RETRY_MS); });
    expect(api.view).toBeNull();
    await act(async () => { await vi.advanceTimersByTimeAsync(VIEW_RETRY_MS); });
    expect(api.view?.spaces).toEqual([]);
    expect(api.error).toBeNull();
    act(() => root.unmount());
  });
});
