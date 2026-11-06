// The Skills tab of the bot panel: the written ways of working the bot learned from its owner. Each row shows the name and
// its one-line description; opening a row reads the steps. Edit changes the description and the steps (the name stays);
// Delete asks first and moves the file to a removed folder. The bot makes skills (when asked); this page never does.
import { useCallback, useEffect, useState } from 'react';
import { plainError } from '../../../lib/potato';
import { SKILLS_COPY, SKILL_LIMITS, skillSize, tauriSkillsTransport, type Skill, type SkillsTransport } from '../../../lib/potato-skills';
import '../../../styles/cockpit-potato-skills.css';

export interface SkillsTabProps {
  bot: { id: string; name: string };
  transport?: SkillsTransport;
  onNotify?: (text: string) => void;
}

type Mode = { kind: 'read' } | { kind: 'edit'; description: string; body: string } | { kind: 'confirm' };

export function SkillsTab({ bot, transport = tauriSkillsTransport, onNotify }: SkillsTabProps) {
  const [skills, setSkills] = useState<Skill[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>({ kind: 'read' });
  const [busy, setBusy] = useState(false);

  const load = useCallback(() => {
    setError(null);
    transport.list(bot.id).then(setSkills, (e: unknown) => {
      setSkills([]);
      setError(plainError(e, SKILLS_COPY.loadFailed));
    });
  }, [bot.id, transport]);

  useEffect(() => {
    setSkills(null);
    setOpen(null);
    setMode({ kind: 'read' });
    load();
  }, [load]);

  const choose = (slug: string) => {
    setOpen((was) => (was === slug ? null : slug));
    setMode({ kind: 'read' });
    setError(null);
  };

  const save = async (skill: Skill, description: string, body: string) => {
    setBusy(true);
    setError(null);
    try {
      const saved = await transport.save(bot.id, skill.slug, description, body);
      setSkills((was) => (was ?? []).map((s) => (s.slug === saved.slug ? saved : s)));
      setMode({ kind: 'read' });
      onNotify?.(SKILLS_COPY.saved(saved.name));
    } catch (e) {
      setError(plainError(e, SKILLS_COPY.saveFailed));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (skill: Skill) => {
    setBusy(true);
    setError(null);
    try {
      await transport.remove(bot.id, skill.slug);
      setSkills((was) => (was ?? []).filter((s) => s.slug !== skill.slug));
      setOpen(null);
      setMode({ kind: 'read' });
      onNotify?.(SKILLS_COPY.deleted(skill.name));
    } catch (e) {
      setError(plainError(e, SKILLS_COPY.deleteFailed));
      load();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="ck-skills">
      <p className="ck-sub ck-skills-intro">{SKILLS_COPY.intro}</p>
      {skills === null && <p className="ck-sub">{SKILLS_COPY.loading}</p>}
      {error && (
        <p className="ck-sub ck-skills-error" role="alert">
          {error}
        </p>
      )}
      {skills !== null && skills.length === 0 && !error && <p className="ck-sub">{SKILLS_COPY.empty}</p>}
      <ul className="ck-skills-list">
        {(skills ?? []).map((skill) => {
          const expanded = open === skill.slug;
          return (
            <li key={skill.slug} className="ck-skill" data-open={expanded || undefined}>
              <button type="button" className="ck-skill-head" aria-expanded={expanded} onClick={() => choose(skill.slug)}>
                <span className="ck-skill-name">{skill.name}</span>
                <span className="ck-sub ck-skill-desc">{skill.description || SKILLS_COPY.noDescription}</span>
              </button>
              {expanded && mode.kind === 'read' && (
                <div className="ck-skill-body">
                  <pre className="ck-skill-steps">{skill.body}</pre>
                  <div className="ck-skill-actions">
                    <button type="button" className="ck-btn ck-btn--secondary ck-btn--sm" onClick={() => setMode({ kind: 'edit', description: skill.description, body: skill.body })}>
                      {SKILLS_COPY.edit}
                    </button>
                    <button type="button" className="ck-btn ck-btn--secondary ck-btn--sm" onClick={() => setMode({ kind: 'confirm' })}>
                      {SKILLS_COPY.delete}
                    </button>
                  </div>
                </div>
              )}
              {expanded && mode.kind === 'edit' && (
                <form
                  className="ck-skill-body"
                  onSubmit={(e) => {
                    e.preventDefault();
                    void save(skill, mode.description, mode.body);
                  }}
                >
                  <label className="ck-skill-label">
                    {SKILLS_COPY.description}
                    <input
                      className="ck-skill-input"
                      value={mode.description}
                      maxLength={SKILL_LIMITS.description}
                      onChange={(e) => setMode({ ...mode, description: e.target.value })}
                    />
                  </label>
                  <label className="ck-skill-label">
                    {SKILLS_COPY.steps}
                    <textarea className="ck-skill-textarea" rows={8} value={mode.body} onChange={(e) => setMode({ ...mode, body: e.target.value })} />
                  </label>
                  {skillSize(skill.name, mode.description, mode.body) > SKILL_LIMITS.file && (
                    <p className="ck-sub ck-skills-error" role="alert">
                      {SKILLS_COPY.tooLong(skillSize(skill.name, mode.description, mode.body))}
                    </p>
                  )}
                  <div className="ck-skill-actions">
                    <button type="submit" className="ck-btn ck-btn--primary ck-btn--sm" disabled={busy || mode.body.trim() === '' || skillSize(skill.name, mode.description, mode.body) > SKILL_LIMITS.file}>
                      {SKILLS_COPY.save}
                    </button>
                    <button type="button" className="ck-btn ck-btn--secondary ck-btn--sm" onClick={() => setMode({ kind: 'read' })}>
                      {SKILLS_COPY.cancel}
                    </button>
                  </div>
                </form>
              )}
              {expanded && mode.kind === 'confirm' && (
                <div className="ck-skill-body" role="group" aria-label={SKILLS_COPY.delete}>
                  <p className="ck-sub">{SKILLS_COPY.confirm(skill.name)}</p>
                  <div className="ck-skill-actions">
                    <button type="button" className="ck-btn ck-btn--primary ck-btn--sm" disabled={busy} onClick={() => void remove(skill)}>
                      {SKILLS_COPY.confirmYes}
                    </button>
                    <button type="button" className="ck-btn ck-btn--secondary ck-btn--sm" onClick={() => setMode({ kind: 'read' })}>
                      {SKILLS_COPY.cancel}
                    </button>
                  </div>
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
