// The two sub-pages of Settings > Potato Bot, linked from its page as rows: "Models and fallbacks" and "Usage". A sub-page
// opens over the Settings window (a full-window layer with a Back button; Escape goes back too) and closes to the same row.
// Nothing here saves anything itself: the pages read and save through the Potato settings store and the usage command.
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { brainLabel, type PotatoTransport } from '../../../lib/potato';
import type { PotatoUsageTransport } from '../../../lib/potato-usage';
import { tauriUsageTransport } from '../../../lib/potato-usage';
import '../../../styles/potato-settings-pages.css';
import type { RowDef } from '../model';
import { Btn, SettingRow } from '../settings-ui';
import { ModelsPage } from './PotatoModelsPage';
import { usePotatoSettings } from './usePotatoSettings';
import { UsagePage } from './PotatoUsagePage';
import { ConnectorsSection } from './ConnectorsSection';
import { McpServersSection } from './McpServersSection';
import { tauriMcpTransport, type McpTransport } from '../../../lib/potato-mcp';

/** The layer a sub-page opens in. Back, the Escape key and a click on the dimmed part all close it. */
export function SubPageSheet({ title, onClose, children }: { title: string; onClose: () => void; children: ReactNode }) {
  const back = useRef<HTMLButtonElement>(null);
  useEffect(() => back.current?.focus(), []);
  return (
    <div
      className="pot-sub-scrim"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') onClose();
      }}
    >
      <div className="pot-sub" role="dialog" aria-modal="true" aria-labelledby="pot-sub-title">
        <header className="pot-sub-head">
          <button ref={back} type="button" className="st-btn is-small pot-sub-back" onClick={onClose}>
            ‹ Back
          </button>
          <h2 id="pot-sub-title">{title}</h2>
        </header>
        <div className="pot-sub-body">{children}</div>
      </div>
    </div>
  );
}

function ModelsRow({ transport }: { transport: PotatoTransport }) {
  const [open, setOpen] = useState(false);
  const { settings } = usePotatoSettings(transport);
  const fallbacks = settings?.fallbacks ?? [];
  const summary = settings
    ? `Talk brain: ${brainLabel(settings.brain)}. ${fallbacks.length ? `If it fails, try ${fallbacks.map(brainLabel).join(', then ')}.` : 'No fallback.'}`
    : 'Loading…';
  return (
    <SettingRow label="Models and fallbacks" description={summary}>
      <Btn aria-label="Open Models and fallbacks" onClick={() => setOpen(true)}>
        Open
      </Btn>
      {open && (
        <SubPageSheet title="Models and fallbacks" onClose={() => setOpen(false)}>
          <ModelsPage transport={transport} />
        </SubPageSheet>
      )}
    </SettingRow>
  );
}

function UsageRow({ usage }: { usage: PotatoUsageTransport }) {
  const [open, setOpen] = useState(false);
  return (
    <SettingRow label="Usage" description="Messages, tokens and cost for the last 30 days, and today against the limits.">
      <Btn aria-label="Open Usage" onClick={() => setOpen(true)}>
        Open
      </Btn>
      {open && (
        <SubPageSheet title="Usage" onClose={() => setOpen(false)}>
          <UsagePage transport={usage} />
        </SubPageSheet>
      )}
    </SettingRow>
  );
}

/** Opens MCP servers for the first bot the Potato page lists. */
function McpRow({ transport, mcp }: { transport: PotatoTransport; mcp: McpTransport }) {
  const [open, setOpen] = useState(false);
  const [botId, setBotId] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    transport.state().then((s) => live && setBotId(s.bots[0]?.id ?? null), () => undefined);
    return () => {
      live = false;
    };
  }, [transport]);
  return (
    <SettingRow label="MCP servers" description="Tools that let the bot use programs on its computer.">
      <Btn aria-label="Open MCP servers" onClick={() => setOpen(true)}>
        Open
      </Btn>
      {open && (
        <SubPageSheet title="MCP servers" onClose={() => setOpen(false)}>
          {botId ? <McpServersSection botId={botId} transport={mcp} /> : <p>Make a bot first, then add its tools here.</p>}
        </SubPageSheet>
      )}
    </SettingRow>
  );
}

/** Opens Connectors for the first bot the Potato page lists: the services, then Add your own. */
function ConnectorsRow({ transport, mcp }: { transport: PotatoTransport; mcp: McpTransport }) {
  const [open, setOpen] = useState(false);
  const [botId, setBotId] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    transport.state().then((s) => live && setBotId(s.bots[0]?.id ?? null), () => undefined);
    return () => {
      live = false;
    };
  }, [transport]);
  return (
    <SettingRow label="Connectors" description="Let the bot use a service you have, like GitHub or Notion. You connect it here; the bot never asks for the token.">
      <Btn aria-label="Open Connectors" onClick={() => setOpen(true)}>
        Open
      </Btn>
      {open && (
        <SubPageSheet title="Connectors" onClose={() => setOpen(false)}>
          {botId ? (
            <>
              <ConnectorsSection botId={botId} transport={mcp} />
              <McpServersSection botId={botId} transport={mcp} />
            </>
          ) : (
            <p>Make a bot first, then connect services here.</p>
          )}
        </SubPageSheet>
      )}
    </SettingRow>
  );
}

/** The rows that open the sub-pages, for Potato Bot's page. */
export function makePotatoSubPageRows(transport: PotatoTransport, usage: PotatoUsageTransport = tauriUsageTransport, mcp: McpTransport = tauriMcpTransport): RowDef[] {
  return [
    {
      id: 'potato-models',
      label: 'Models and fallbacks',
      hint: 'The talk brain, and the brains to try when it is busy or down.',
      keywords: 'potato talk brain model models deepseek opencode go agents fallback fallbacks backup provider order limit down',
      group: 'Potato Bot',
      render: () => <ModelsRow transport={transport} />,
    },
    {
      id: 'potato-usage',
      label: 'Usage',
      hint: 'Messages, tokens and cost for the last 30 days, and today against the limits.',
      keywords: 'potato usage cost tokens messages spend money sandbox actions day history bill',
      group: 'Potato Bot',
      render: () => <UsageRow usage={usage} />,
    },
    {
      id: 'potato-connectors',
      label: 'Connectors',
      hint: 'Let the bot use a service you have, like GitHub or Notion. You connect it here; the bot never asks for the token.',
      keywords: 'potato connectors connect services github notion gmail slack linear token account app apps integration mcp',
      group: 'Potato Bot',
      render: () => <ConnectorsRow transport={transport} mcp={mcp} />,
    },
    {
      id: 'potato-mcp',
      label: 'MCP servers',
      hint: 'Tools that let the bot use programs on its computer.',
      keywords: 'potato mcp servers tools model context protocol filesystem memory playwright computer plugin',
      group: 'Potato Bot',
      render: () => <McpRow transport={transport} mcp={mcp} />,
    },
  ];
}
