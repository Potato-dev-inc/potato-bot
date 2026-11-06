// State for the Potato Bot cockpit: panes, the selected helper pane, prompts, theme and toasts.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { usePaneRegistry, type PaneRegistryApi } from './usePaneRegistry';
import { agentLabel, useAgentRegistry } from '../lib/agentRegistry';
import { cockpitApi, type PanePrompt } from '../lib/cockpit-api';
import { applyTheme, loadSettings, patchSettings, type AppTheme } from '../lib/settings';
import { subscribeSettingsTheme } from '../lib/settings-sync';
import { useQuestions, type QuestionsApi } from './useQuestions';
import { paneNames } from '../components/cockpit/model';
import type { UserGesture } from '../lib/user-gesture';

const ANSWER_SETTLE_MS = 2_000;

export interface Toast {
  id: number;
  text: string;
  action?: { label: string; run: () => void };
}

export interface CockpitApi {
  asked: QuestionsApi;
  registry: PaneRegistryApi;
  names: Map<string, string>;
  theme: AppTheme;
  selectedId: string | null;
  select: (paneId: string | null) => void;
  prompts: Record<string, PanePrompt | null>;
  statusSince: Map<string, number>;
  now: number;
  toasts: Toast[];
  notify: (text: string, action?: Toast['action']) => void;
  toggleTheme: () => Promise<void>;
  newAgent: (agentId: string, accountId?: string, isolated?: boolean, projectPath?: string) => Promise<string | null>;
  answer: (paneId: string, promptId: string, choiceId: string, label: string, gesture: UserGesture) => Promise<void>;
  interrupt: (paneId: string) => Promise<void>;
  closePane: (paneId: string) => Promise<void>;
  rename: (paneId: string, title: string) => Promise<void>;
  openSettings: (tab?: string) => Promise<void>;
}

function errText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function useCockpit(): CockpitApi {
  const registry = usePaneRegistry();
  const listing = useAgentRegistry();
  const { paneList, panes, initialReady } = registry;

  const [theme, setTheme] = useState<AppTheme>('dark');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [prompts, setPrompts] = useState<Record<string, PanePrompt | null>>({});
  const [statusSince, setStatusSince] = useState<Map<string, number>>(new Map());
  const [now, setNow] = useState(() => Date.now());
  const [toasts, setToasts] = useState<Toast[]>([]);

  const toastId = useRef(0);
  const lastStatus = useRef(new Map<string, string>());

  const notify = useCallback((text: string, action?: Toast['action']) => {
    const id = ++toastId.current;
    const dismiss = () => setToasts((t) => t.filter((x) => x.id !== id));
    const wrapped = action && { label: action.label, run: () => (dismiss(), action.run()) };
    setToasts((t) => [...t.slice(-2), { id, text, action: wrapped }]);
    window.setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), action ? 12000 : 3200);
  }, []);

  const names = useMemo(() => paneNames(paneList, (id) => agentLabel(listing, id)), [paneList, listing]);

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 5000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    const prev = lastStatus.current;
    const changed: string[] = [];
    const seen = new Set<string>();
    for (const p of paneList) {
      seen.add(p.info.id);
      if (prev.get(p.info.id) !== p.status) changed.push(p.info.id);
      prev.set(p.info.id, p.status);
    }
    let gone = false;
    for (const id of [...prev.keys()]) {
      if (!seen.has(id)) {
        prev.delete(id);
        gone = true;
      }
    }
    if (!changed.length && !gone) return;
    const at = Date.now();
    setStatusSince((old) => {
      const next = new Map<string, number>();
      for (const p of paneList) {
        const id = p.info.id;
        next.set(id, changed.includes(id) ? at : (old.get(id) ?? at));
      }
      return next;
    });
  }, [paneList]);

  useEffect(() => {
    let disposed = false;
    loadSettings()
      .then((s) => {
        if (disposed) return;
        setTheme(s.theme ?? 'dark');
        applyTheme(s.theme ?? 'dark');
      })
      .catch(() => {});
    const offTheme = subscribeSettingsTheme((next) => {
      setTheme(next);
      applyTheme(next);
    });
    return () => {
      disposed = true;
      offTheme();
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let off: (() => void) | undefined;
    void cockpitApi
      .onPrompt((e) => setPrompts((p) => ({ ...p, [e.pane_id]: e.prompt })))
      .then((u) => (disposed ? u() : (off = u)));
    return () => {
      disposed = true;
      off?.();
    };
  }, []);

  const requested = useRef(new Set<string>());
  useEffect(() => {
    for (const p of paneList) {
      const id = p.info.id;
      if (p.status !== 'waiting_input') {
        requested.current.delete(id);
        continue;
      }
      if (id in prompts || requested.current.has(id)) continue;
      requested.current.add(id);
      cockpitApi
        .panePrompt(id)
        .then((prompt) => setPrompts((cur) => (id in cur ? cur : { ...cur, [id]: prompt })))
        .catch(() => setPrompts((cur) => (id in cur ? cur : { ...cur, [id]: null })));
    }
    setPrompts((cur) => {
      const keys = Object.keys(cur);
      const stale = keys.filter((k) => panes.get(k)?.status !== 'waiting_input');
      if (!stale.length) return cur;
      const next = { ...cur };
      for (const k of stale) delete next[k];
      return next;
    });
  }, [paneList, panes, prompts]);

  useEffect(() => {
    const want = new URLSearchParams(window.location.search).get('select');
    if (!want || !panes.has(want)) return;
    setSelectedId(want);
  }, [panes]);

  useEffect(() => {
    let disposed = false;
    let off: (() => void) | undefined;
    void cockpitApi
      .onSelect((e) => setSelectedId(e.pane_id))
      .then((u) => (disposed ? u() : (off = u)));
    return () => {
      disposed = true;
      off?.();
    };
  }, []);

  useEffect(() => {
    if (!selectedId || !initialReady || panes.has(selectedId)) return;
    const t = window.setTimeout(() => setSelectedId(null), 1500);
    return () => window.clearTimeout(t);
  }, [selectedId, initialReady, panes]);

  const select = useCallback((id: string | null) => setSelectedId(id), []);

  const toggleTheme = useCallback(async () => {
    const next: AppTheme = theme === 'dark' ? 'light' : 'dark';
    setTheme(next);
    applyTheme(next);
    try {
      await patchSettings({ theme: next });
    } catch (err) {
      notify(`Theme not saved: ${errText(err)}`);
    }
  }, [theme, notify]);

  const newAgent = useCallback(
    async (agentId: string) => {
      try {
        return await registry.spawnPane({ agent_type: agentId, cols: 120, rows: 32 });
      } catch (err) {
        notify(errText(err));
        return null;
      }
    },
    [registry, notify],
  );

  const answering = useRef(new Set<string>());
  const answer = useCallback(
    async (paneId: string, promptId: string, choiceId: string, label: string, gesture: UserGesture) => {
      const key = `${paneId}\n${promptId}`;
      if (answering.current.has(key)) return;
      answering.current.add(key);
      try {
        await cockpitApi.answerPanePrompt(paneId, promptId, choiceId, gesture);
        notify(`Answered ${names.get(paneId) ?? 'agent'}: ${label}`);
        window.setTimeout(() => answering.current.delete(key), ANSWER_SETTLE_MS);
      } catch (err) {
        answering.current.delete(key);
        notify(errText(err));
      }
    },
    [names, notify],
  );

  const interrupt = useCallback(
    async (paneId: string) => {
      try {
        await cockpitApi.interruptPane(paneId);
        notify('Sent Ctrl+C');
      } catch (err) {
        notify(errText(err));
      }
    },
    [notify],
  );

  const closePane = useCallback(
    async (paneId: string) => {
      try {
        await registry.killPane(paneId);
        setSelectedId((cur) => (cur === paneId ? null : cur));
      } catch (err) {
        notify(errText(err));
      }
    },
    [registry, notify],
  );

  const rename = useCallback(
    async (paneId: string, title: string) => {
      try {
        await cockpitApi.paneSetTitle(paneId, title);
        await registry.refresh();
      } catch (err) {
        notify(errText(err));
      }
    },
    [registry, notify],
  );

  const openSettings = useCallback(async () => {
    // The cockpit overrides this to switch its own view; the standalone helper keeps Tauri's.
    try {
      await cockpitApi.openSettings('potato');
    } catch (err) {
      notify(`Settings did not open: ${errText(err)}`);
    }
  }, [notify]);

  const asked = useQuestions(notify, now);

  return {
    asked,
    registry,
    names,
    theme,
    selectedId,
    select,
    prompts,
    statusSince,
    now,
    toasts,
    notify,
    toggleTheme,
    newAgent,
    answer,
    interrupt,
    closePane,
    rename,
    openSettings,
  };
}
