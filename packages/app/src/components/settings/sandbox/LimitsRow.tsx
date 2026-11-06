// Settings > Potato Bot > Sandbox: "CPU limit" and "Memory limit". Two sliders that apply at once (no restart of the sandbox),
// with what the bot's computer uses now. The defaults suit an ordinary PC: half the processors, a quarter of the memory.
import { useCallback, useEffect, useRef, useState } from 'react';
import { plainSandboxError, type MemoryReturn, type SandboxLimits, type SandboxTransport } from '../../../lib/potato-sandbox';
import type { SettingsEnv } from '../model';
import { Btn, Note, SettingRow } from '../settings-ui';

/** How often the "in use now" numbers are read while the page is open. */
export const LIMITS_POLL_MS = 3000;
/** A slider drag is sent this long after the last move. */
export const LIMITS_SEND_MS = 250;

export const gb = (mb: number) => (mb / 1024).toFixed(mb % 1024 === 0 ? 0 : 1);

export function LimitsRow({ transport, env }: { transport: SandboxTransport; env: SettingsEnv }) {
  const [limits, setLimits] = useState<SandboxLimits | null>(null);
  const [cpu, setCpu] = useState<number | null>(null);
  const [mem, setMem] = useState<number | null>(null);
  const timer = useRef<number | null>(null);
  const dragging = useRef(false);

  const read = useCallback(async () => {
    try {
      const next = await transport.limitsGet();
      setLimits(next);
      if (!dragging.current) {
        setCpu(next.cpuPercent);
        setMem(next.memoryMb);
      }
    } catch {
      // The page keeps what it has; the next read tries again.
    }
  }, [transport]);

  useEffect(() => {
    void read();
    const id = window.setInterval(() => void read(), LIMITS_POLL_MS);
    return () => {
      window.clearInterval(id);
      if (timer.current) window.clearTimeout(timer.current);
    };
  }, [read]);

  const send = (nextCpu: number, nextMem: number) => {
    dragging.current = true;
    if (timer.current) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(async () => {
      env.setError(null);
      try {
        setLimits(await transport.limitsSet(nextCpu, nextMem));
        env.markSaved();
      } catch (error) {
        env.setError(plainSandboxError(error, 'Could not set that limit.'));
      } finally {
        dragging.current = false;
      }
    }, LIMITS_SEND_MS);
  };

  if (!limits || cpu === null || mem === null) {
    return <SettingRow label="Computer limits" description="How much of this PC the bot's computer may use." />;
  }
  const memMax = Math.max(limits.maxMemoryMb, limits.defaultMemoryMb);
  const isDefault = cpu === limits.defaultCpuPercent && mem === limits.defaultMemoryMb;
  return (
    <SettingRow
      label="Computer limits"
      description="How much of this PC the bot's computer may use. A change applies at once, even while it runs. Heavy jobs run slower with lower limits."
    >
      <div className="sbx-limits">
        <label className="sbx-limit">
          <span className="sbx-limit-name">CPU limit</span>
          <input
            type="range"
            aria-label="CPU limit, percent of this PC"
            min={10}
            max={100}
            step={5}
            value={cpu}
            onChange={(e) => {
              const next = Number(e.currentTarget.value);
              setCpu(next);
              send(next, mem);
            }}
          />
          <span className="sbx-limit-value">{cpu}% of this PC</span>
        </label>
        <label className="sbx-limit">
          <span className="sbx-limit-name">Memory limit</span>
          <input
            type="range"
            aria-label="Memory limit, gigabytes"
            min={1024}
            max={memMax}
            step={256}
            value={Math.min(mem, memMax)}
            onChange={(e) => {
              const next = Number(e.currentTarget.value);
              setMem(next);
              send(cpu, next);
            }}
          />
          <span className="sbx-limit-value">{gb(mem)} GB</span>
        </label>
        <p className="sbx-hint" data-testid="limits-use">
          {limits.running
            ? `In use now: ${limits.usedCpuPercent === null ? '…' : `${limits.usedCpuPercent}%`} CPU, ${limits.usedMemoryMb === null ? '…' : `${gb(limits.usedMemoryMb)} GB`} memory.`
            : "The bot's computer is not running, so nothing is in use. The limits apply the next time it starts."}
        </p>
        {limits.memoryPressure && <Note>The memory limit is reached, so the computer is being slowed down to stay inside it. Raise it if jobs feel stuck.</Note>}
        {!isDefault && (
          <Btn
            onClick={() => {
              setCpu(limits.defaultCpuPercent);
              setMem(limits.defaultMemoryMb);
              send(limits.defaultCpuPercent, limits.defaultMemoryMb);
            }}
          >
            Use the suggested limits ({limits.defaultCpuPercent}%, {gb(limits.defaultMemoryMb)} GB)
          </Btn>
        )}
      </div>
    </SettingRow>
  );
}

/** "Give unused memory back to Windows": adds autoMemoryReclaim=gradual to the person's .wslconfig, only when they press the button. */
export function MemoryReturnRow({ transport, env }: { transport: SandboxTransport; env: SettingsEnv }) {
  const [state, setState] = useState<MemoryReturn | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let live = true;
    transport.memoryReturnGet().then(
      (next) => live && setState(next),
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [transport]);
  return (
    <SettingRow
      label="Give unused memory back to Windows"
      description="Windows Task Manager shows the bot's computer as VmmemWSL and it keeps memory it no longer needs. This asks WSL to hand unused memory back gradually. It edits only one line of your .wslconfig file (a copy is kept) and takes effect the next time WSL starts, for example after you restart Windows."
    >
      {state?.state === 'on' && <Note>On. It takes effect the next time WSL starts (restart Windows, or close all WSL programs and wait a minute).</Note>}
      {state?.state === 'other' && <Note>Your .wslconfig already sets this to "{state.value}". It is left as you chose.</Note>}
      {state?.state === 'off' && (
        <Btn
          disabled={busy}
          onClick={async () => {
            setBusy(true);
            env.setError(null);
            try {
              setState(await transport.memoryReturnEnable());
              env.markSaved();
            } catch (error) {
              env.setError(plainSandboxError(error, 'Could not change your .wslconfig file.'));
            } finally {
              setBusy(false);
            }
          }}
        >
          Turn on
        </Btn>
      )}
    </SettingRow>
  );
}
