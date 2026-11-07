import { describe, expect, it } from 'vitest';
import { GO_MODELS, goKeyShapeProblem, goModelLabel } from './potato-go-models';

describe('the Go plan models and key shape', () => {
  it('lists each model once, by the id the plan uses', () => {
    const ids = GO_MODELS.map((m) => m.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids.every((id) => /^[a-z0-9.-]+$/.test(id))).toBe(true);
    expect(goModelLabel('glm-5.3')).toBe('GLM-5.3');
    expect(goModelLabel('brand-new-model')).toBe('brand-new-model');
  });

  it('refuses only a shape that cannot be a key, never a prefix', () => {
    expect(goKeyShapeProblem('')).toMatch(/Paste/);
    expect(goKeyShapeProblem('a b')).toMatch(/space/);
    expect(goKeyShapeProblem('https://opencode.ai')).toMatch(/web address/);
    expect(goKeyShapeProblem('short')).toMatch(/too short/);
    for (const key of ['oc-0123456789abcdef', 'sk-0123456789abcdef', 'anything0123456789']) expect(goKeyShapeProblem(key)).toBeNull();
  });
});
