import { describe, expect, it } from 'vitest';
import { isVersionNewer, normalizeVersion, parseVersionParts, plainUpdateError, runningJobWarning } from './app-update';

describe('version compare', () => {
  it('reads v-prefixed and plain versions', () => {
    expect(normalizeVersion(' v1.0.3 ')).toBe('1.0.3');
    expect(parseVersionParts('1.0.3')).toEqual([1, 0, 3]);
  });

  it('a higher number is newer, an equal or lower one is not', () => {
    expect(isVersionNewer('1.0.3', '1.0.2')).toBe(true);
    expect(isVersionNewer('1.1.0', '1.0.9')).toBe(true);
    expect(isVersionNewer('2.0.0', '1.9.9')).toBe(true);
    expect(isVersionNewer('1.0.2', '1.0.2')).toBe(false);
    expect(isVersionNewer('1.0.1', '1.0.2')).toBe(false);
    expect(isVersionNewer('1.0', '1.0.0')).toBe(false);
    expect(isVersionNewer('1.0.10', '1.0.9')).toBe(true);
  });
});

describe('plain errors', () => {
  it('keeps the backend sentence and drops its code', () => {
    expect(plainUpdateError(new Error('Could not reach the update server. Check your internet.'))).toBe(
      'Could not reach the update server. Check your internet.',
    );
    expect(plainUpdateError('UPDATE_JOB_RUNNING: A bot is working on a job.')).toBe('A bot is working on a job.');
  });

  it('turns anything that is not a sentence for a person into the safe one', () => {
    for (const raw of [undefined, '', new Error('C:\\Users\\x\\a.msi failed'), new Error('TypeError: x is not a function')]) {
      expect(plainUpdateError(raw)).toContain('Your current version keeps working');
    }
  });

  it('says one bot or several when a job runs', () => {
    expect(runningJobWarning(['fish'])).toContain('A bot is working on a job');
    expect(runningJobWarning(['a', 'b'])).toContain('2 bots are working on a job');
  });
});
