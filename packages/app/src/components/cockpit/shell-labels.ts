// Words for the cockpit shell: a project is called by its folder's name.

/** Where Puppet Master keeps a separate copy: `<project>/.puppet-master/worktrees/<id>`. */
const COPY_MARK = '/.puppet-master/worktrees/';

/** True when `cwd` is the folder of a separate copy (its last part is an id, not a project name). */
export function isSeparateCopyCwd(cwd: string | undefined | null): boolean {
  return !!cwd && cwd.replace(/\\/g, '/').indexOf(COPY_MARK) > 0;
}

function folderName(cwd: string | undefined | null): string {
  if (!cwd) return '';
  const parts = cwd.replace(/\\/g, '/').split('/').filter(Boolean);
  return parts[parts.length - 1] ?? cwd;
}

/**
 * The project a pane works in, by name: `shop`, never the `7d4cb2b3-2-209c` folder of its separate copy.
 */
export function projectLabel(cwd: string | undefined | null): string {
  return folderName(cwd);
}

/** `shop` or `separate copy of shop`: the folder the agent works in, or the copy of it. */
export function projectWithCopy(cwd: string | undefined | null): string {
  const name = projectLabel(cwd);
  return isSeparateCopyCwd(cwd) ? `separate copy of ${name}` : name;
}

/** The real project folder behind a pane's working directory, for a hover title. */
export function realFolderPath(cwd: string | undefined | null): string | undefined {
  return cwd ?? undefined;
}
