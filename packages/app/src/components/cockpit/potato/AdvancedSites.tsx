// "Sites" on the Advanced tab: an optional list of the only sites the bot's web tools may open. Off by default. The owner sets it
// here and nowhere else (no tool the bot can call reaches it). The section says plainly what the list does not cover.
import { useEffect, useState } from 'react';
import { SITES_COPY, type SiteLimit } from '../../../lib/potato-botsettings';
import { AdvancedSection } from './AdvancedSection';

export interface AdvancedSitesProps {
  limit: SiteLimit;
  /** Saves the switch and the list. Resolves true when it was saved; a failed save is the page's to show. */
  onSave: (limit: SiteLimit) => Promise<boolean>;
}

const sitesOf = (text: string) =>
  text
    .split(/[\n,]/)
    .map((s) => s.trim())
    .filter(Boolean);

export function AdvancedSites({ limit, onSave }: AdvancedSitesProps) {
  const saved = limit.sites.join('\n');
  const [draft, setDraft] = useState(saved);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  // The list in use changes only by a save: then the box shows it.
  useEffect(() => setDraft(saved), [saved]);

  const dirty = sitesOf(draft).join('\n') !== saved;
  const run = async (next: SiteLimit, done: string) => {
    setBusy(true);
    setStatus(null);
    const ok = await onSave(next);
    setBusy(false);
    setStatus(ok ? done : null);
  };
  const emptyOn = limit.enabled && limit.sites.length === 0;

  return (
    <AdvancedSection title={SITES_COPY.title} line={SITES_COPY.line} id="ck-adv-sites">
      <div className="ck-potato-adv-switch-row">
        <div className="ck-potato-adv-switch-text">
          <span id="ck-adv-sites-switch" className="ck-potato-adv-switch-label">
            {SITES_COPY.switchLabel}
          </span>
          <span className="ck-potato-adv-line">{limit.enabled ? SITES_COPY.onLine : SITES_COPY.offLine}</span>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={limit.enabled}
          aria-labelledby="ck-adv-sites-switch"
          disabled={busy}
          className={`ck-potato-switch${limit.enabled ? ' is-on' : ''}`}
          onClick={() => void run({ enabled: !limit.enabled, sites: limit.sites }, limit.enabled ? SITES_COPY.switchedOff : SITES_COPY.switchedOn)}
        >
          <span className="ck-potato-adv-sr">{limit.enabled ? 'On' : 'Off'}</span>
        </button>
      </div>
      <textarea
        className="ck-potato-adv-input ck-potato-adv-text"
        rows={4}
        value={draft}
        placeholder={SITES_COPY.placeholder}
        aria-label={SITES_COPY.listLabel}
        spellCheck={false}
        onChange={(event) => {
          setDraft(event.target.value);
          setStatus(null);
        }}
      />
      <div className="ck-potato-adv-row">
        <span className="ck-potato-adv-line">{SITES_COPY.subdomains}</span>
        <div className="ck-potato-adv-actions">
          <button
            type="button"
            className="ck-btn ck-btn--primary ck-btn--sm"
            disabled={!dirty || busy}
            onClick={() => void run({ enabled: limit.enabled, sites: sitesOf(draft) }, SITES_COPY.saved)}
          >
            {SITES_COPY.save}
          </button>
        </div>
      </div>
      {emptyOn && (
        <p className="ck-potato-adv-status" role="note">
          {SITES_COPY.emptyOn}
        </p>
      )}
      {status && (
        <p className="ck-potato-adv-status" role="status">
          {status}
        </p>
      )}
      <p className="ck-potato-adv-line ck-potato-adv-cannot">
        <strong>{SITES_COPY.cannotTitle}</strong> {SITES_COPY.cannot}
      </p>
    </AdvancedSection>
  );
}
