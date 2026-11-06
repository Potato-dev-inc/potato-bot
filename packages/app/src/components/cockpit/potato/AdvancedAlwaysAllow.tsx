// "Always allowed" on the Advanced tab: the rules the owner saved with the Always allow button on an approval card, one row
// each (the bot's name and the kind of command in plain words) with a Remove button. Removing takes effect at once: the
// next command of that kind asks again. Rules are made only from the card, never here.
import type { AlwaysRule } from '../../../lib/potato-botsettings';
import { AdvancedSection } from './AdvancedSection';

export const ALWAYS_ALLOW_COPY = {
  title: 'Always allowed',
  line: 'Kinds of command this bot runs without asking, because you chose Always allow on a card. Removing one makes it ask again at once.',
  none: 'Nothing is always allowed. The bot asks on every card.',
  remove: 'Remove',
};

export interface AdvancedAlwaysAllowProps {
  botName: string;
  rules: readonly AlwaysRule[];
  /** Removes the rule for this class. A failed save is the page's to show. */
  onRemove: (cls: string) => void;
}

export function AdvancedAlwaysAllow({ botName, rules, onRemove }: AdvancedAlwaysAllowProps) {
  return (
    <AdvancedSection title={ALWAYS_ALLOW_COPY.title} line={ALWAYS_ALLOW_COPY.line} id="ck-adv-always-allow">
      {rules.length === 0 ? (
        <p className="ck-potato-adv-line">{ALWAYS_ALLOW_COPY.none}</p>
      ) : (
        <ul className="ck-potato-adv-list">
          {rules.map((rule) => (
            <li key={rule.class} className="ck-potato-adv-row">
              <span className="ck-potato-adv-line">
                {botName} may run a command that {rule.description}, without asking.
              </span>
              <button type="button" className="ck-btn ck-btn--plain" onClick={() => onRemove(rule.class)}>
                {ALWAYS_ALLOW_COPY.remove}
              </button>
            </li>
          ))}
        </ul>
      )}
    </AdvancedSection>
  );
}
