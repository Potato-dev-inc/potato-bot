// "When to ask" on the Advanced tab (Professional mode): how much the bot does on its own before it stops for a card.
// Three levels, one plain line each. Passwords and logins always come to the person, at every level.
import type { BotLevel } from '../../../lib/potato-botsettings';
import { AdvancedSection } from './AdvancedSection';

export type AskLevel = BotLevel;

export const ASK_LEVEL_COPY = {
  title: 'When to ask',
  line: 'How much the bot does on its own before it stops with a card for you.',
  never: 'Passwords and logins are always yours to type, at every level.',
};

export const ASK_LEVELS: readonly { id: AskLevel; label: string; line: string }[] = [
  { id: 'ask_first', label: 'Ask first', line: 'Asks before any command that changes things or sends a file. Reading is free.' },
  { id: 'ask_when_risky', label: 'Ask when risky', line: 'Works on its own inside its computer. Asks before anything that leaves it or cannot be undone.' },
  { id: 'full', label: 'Full', line: 'Works on its own inside its computer. Asks only with questions and when a login is missing.' },
];

export interface AdvancedAskLevelProps {
  value: AskLevel;
  /** Saves the level. A failed save is the page's to show. */
  onChange: (level: AskLevel) => void;
  disabled?: boolean;
}

export function AdvancedAskLevel({ value, onChange, disabled = false }: AdvancedAskLevelProps) {
  return (
    <AdvancedSection title={ASK_LEVEL_COPY.title} line={ASK_LEVEL_COPY.line} id="ck-adv-ask-level">
      <fieldset className="ck-potato-radios" disabled={disabled}>
        <legend className="ck-potato-sr">{ASK_LEVEL_COPY.title}</legend>
        {ASK_LEVELS.map((level) => (
          <label key={level.id} className="ck-potato-radio">
            <input type="radio" name="ck-ask-level" value={level.id} checked={value === level.id} onChange={() => onChange(level.id)} />
            <span>
              <span className="ck-potato-adv-switch-label">{level.label}</span>
              <span className="ck-potato-adv-line">{level.line}</span>
            </span>
          </label>
        ))}
      </fieldset>
      <p className="ck-potato-adv-line">{ASK_LEVEL_COPY.never}</p>
    </AdvancedSection>
  );
}
