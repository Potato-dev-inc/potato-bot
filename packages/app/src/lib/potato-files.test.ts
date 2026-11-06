import { describe, expect, it } from 'vitest';
import { childPath, crumbs, fileSize, HOME_PATH, parentPath } from './potato-files';

describe('the paths the Files section builds', () => {
  it('joins a folder and a name, from home or from deeper down', () => {
    expect(childPath(HOME_PATH, 'docs')).toBe('~/docs');
    expect(childPath('~/docs', 'plan.md')).toBe('~/docs/plan.md');
  });

  it('goes up one folder and stops at home', () => {
    expect(parentPath('~/docs/reports')).toBe('~/docs');
    expect(parentPath('~/docs')).toBe('~');
    expect(parentPath('~')).toBe('~');
  });

  it('makes one crumb per folder, each with the path it opens', () => {
    expect(crumbs('~')).toEqual([{ label: 'Home', path: '~' }]);
    expect(crumbs('~/docs/reports')).toEqual([
      { label: 'Home', path: '~' },
      { label: 'docs', path: '~/docs' },
      { label: 'reports', path: '~/docs/reports' },
    ]);
  });

  it('never makes a path with .. from what the computer lists', () => {
    for (const path of ['~', '~/a', '~/a/b']) {
      expect(parentPath(path)).not.toContain('..');
      for (const crumb of crumbs(path)) expect(crumb.path).not.toContain('..');
    }
  });
});

describe('file sizes', () => {
  it('uses the unit a person expects, from bytes up', () => {
    expect([0, 12, 1023, 1024, 1536, 2048, 900_000, 10 * 1024 * 1024, 1.5 * 1024 * 1024].map(fileSize)).toEqual(['0 B', '12 B', '1023 B', '1 KB', '1.5 KB', '2 KB', '879 KB', '10 MB', '1.5 MB']);
  });
});
