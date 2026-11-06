// The update flow of the page: a quiet check, the offer, a warning when a job runs, the press, the plain error.
import { useCallback, useEffect, useRef, useState } from 'react';
import { plainUpdateError, tauriUpdateTransport, type UpdateOffer, type UpdateTransport } from '../../lib/app-update';

export type UpdatePhase = 'idle' | 'checking' | 'none' | 'available' | 'warn' | 'installing' | 'error';

export interface AppUpdateState {
  phase: UpdatePhase;
  offer: UpdateOffer | null;
  busyBots: string[];
  error: string | null;
  /** Looks for an update. A quiet check (loud = false) never shows an error; a press on Check does. */
  check(loud?: boolean): Promise<void>;
  /** The press on Update. With a job running it first asks (phase `warn`); a second press goes on. */
  update(): Promise<void>;
  dismissWarning(): void;
}

/** A quiet check at start and then every six hours. */
export const RECHECK_MS = 6 * 60 * 60 * 1000;

export function useAppUpdate(transport: UpdateTransport = tauriUpdateTransport, auto = true): AppUpdateState {
  const [phase, setPhase] = useState<UpdatePhase>('idle');
  const [offer, setOffer] = useState<UpdateOffer | null>(null);
  const [busyBots, setBusyBots] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const live = useRef(true);
  const installing = useRef(false);

  const check = useCallback(
    async (loud = false) => {
      if (installing.current) return;
      if (loud) {
        setError(null);
        setPhase('checking');
      }
      try {
        const found = await transport.check();
        if (!live.current) return;
        setOffer(found);
        setPhase(found ? 'available' : loud ? 'none' : 'idle');
      } catch (e) {
        if (!live.current) return;
        if (!loud) {
          setPhase((p) => (p === 'checking' ? 'idle' : p));
          return;
        }
        setError(plainUpdateError(e));
        setPhase('error');
      }
    },
    [transport],
  );

  const run = useCallback(
    async (confirmRunning: boolean) => {
      installing.current = true;
      setError(null);
      setPhase('installing');
      try {
        await transport.install(confirmRunning);
        // The installer closes the app; if this page is still here the installer did not start.
      } catch (e) {
        if (!live.current) return;
        setError(plainUpdateError(e));
        setPhase('error');
      } finally {
        installing.current = false;
      }
    },
    [transport],
  );

  const update = useCallback(async () => {
    if (installing.current) return;
    if (phase !== 'warn') {
      let bots: string[] = [];
      try {
        bots = await transport.busy();
      } catch {
        bots = [];
      }
      if (bots.length > 0) {
        setBusyBots(bots);
        setPhase('warn');
        return;
      }
    }
    await run(phase === 'warn');
  }, [phase, run, transport]);

  const dismissWarning = useCallback(() => setPhase(offer ? 'available' : 'idle'), [offer]);

  useEffect(() => {
    live.current = true;
    if (!auto) {
      return () => {
        live.current = false;
      };
    }
    void check(false);
    const timer = window.setInterval(() => void check(false), RECHECK_MS);
    return () => {
      live.current = false;
      window.clearInterval(timer);
    };
  }, [auto, check]);

  return { phase, offer, busyBots, error, check, update, dismissWarning };
}
