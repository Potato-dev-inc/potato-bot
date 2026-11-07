// The OpenCode Go plan: which models a bot can use and what a pasted key must look like.
// Source: https://opencode.ai/docs/go/ . The brain talks to the plan over its OpenAI-compatible chat endpoint
// (`/openai/v1/chat/completions`), so only the models served there are listed. Others on the plan use a different wire
// format and would fail. A model that is saved but not listed here (the plan adds models) still shows and still works.
// Not a catalogue of prices or limits: the plan page says those.

export interface GoModel {
  id: string;
  label: string;
}

export const GO_MODELS: GoModel[] = [
  { id: 'glm-5.3-flash', label: 'GLM-5.3 Flash' },
  { id: 'glm-5.3', label: 'GLM-5.3' },
  { id: 'glm-5.2', label: 'GLM-5.2' },
  { id: 'kimi-k3.1-pro', label: 'Kimi K3.1 Pro' },
  { id: 'kimi-k3', label: 'Kimi K3' },
  { id: 'kimi-k2.7-code', label: 'Kimi K2.7 Code' },
  { id: 'longcat-2.0', label: 'LongCat 2.0' },
  { id: 'deepseek-v4.1-flash', label: 'DeepSeek V4.1 Flash' },
  { id: 'deepseek-v4-pro', label: 'DeepSeek V4 Pro' },
  { id: 'mimo-v2.6-flash', label: 'MiMo V2.6 Flash' },
  { id: 'mimo-v2.6-pro', label: 'MiMo V2.6 Pro' },
  { id: 'mimo-v2.5', label: 'MiMo V2.5' },
  { id: 'mimo-v2.5-pro', label: 'MiMo V2.5 Pro' },
  { id: 'minimax-m3.1-flash', label: 'MiniMax M3.1 Flash' },
  { id: 'hy4', label: 'Hy4' },
  { id: 'hy3', label: 'Hy3' },
  { id: 'step-5-preview', label: 'Step 5 Preview' },
];

/** What the brain uses when no model is chosen (the same default the brain package has). */
export const GO_DEFAULT_MODEL = 'deepseek-v4.1-flash';

/** The label for a model id: the listed name, or the id itself for one the list does not know. */
export function goModelLabel(id: string): string {
  return GO_MODELS.find((m) => m.id === id)?.label ?? id;
}

/**
 * Why a pasted key cannot be right, or null when it may be. Only the shape that is certain: not empty, no spaces or line
 * breaks inside, not a web address. The prefix is NOT checked: the plan's docs do not give one, so a rule on it could lock
 * out a real key. A key the plan does not accept is told by "Check key".
 */
export function goKeyShapeProblem(raw: string): string | null {
  const key = raw.trim();
  if (key === '') return 'Paste your OpenCode Go key.';
  if (/\s/.test(key)) return 'That does not look like an OpenCode Go key: it has a space or a line break inside. Copy it again from the OpenCode console.';
  if (/^https?:\/\//i.test(key)) return 'That is a web address, not a key. Copy the key itself from the OpenCode console.';
  if (key.length < 12) return 'That is too short to be an OpenCode Go key. Copy it again from the OpenCode console.';
  return null;
}
