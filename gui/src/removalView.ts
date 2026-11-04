/**
 * What the Uninstall and Remove dialogs act on, from what the user ticked:
 * kept apart from the page so the rules can be tested.
 */
import type { Cleanup, Format, InstalledView, Remote } from './types';

const SYSTEM_FORMATS: Format[] = ['rpm', 'deb', 'pacman'];

export interface RemovalChoices {
  /** Formats to uninstall: every install unless kept. */
  uninstall: Format[];
  /** Repositories to offer: only while the Flatpak installed from them goes too. */
  remotes: Remote[];
  /** Ticked repositories that are still offered. */
  repos: string[];
  /** Ticked leftovers that are offered. */
  purge: string[];
  /** Said before confirming when a system package is among the uninstalls. */
  systemHint: string | undefined;
}

export function removalChoices(
  installs: InstalledView[],
  keep: Format[],
  cleanup: Cleanup,
  dropRepos: string[],
  purge: string[],
): RemovalChoices {
  const uninstall = installs.map((i) => i.format).filter((f) => !keep.includes(f));
  const remotes = uninstall.includes('flatpak') ? cleanup.remotes : [];
  return {
    uninstall,
    remotes,
    repos: dropRepos.filter((name) => remotes.some((r) => r.name === name)),
    purge: purge.filter((path) => cleanup.leftovers.some((l) => l.path === path)),
    systemHint: uninstall.some((f) => SYSTEM_FORMATS.includes(f))
      ? 'Uninstalling a system package asks for your password.'
      : undefined,
  };
}

/** `list` with `value` in it when `on`, without it otherwise. */
export function toggled<T>(list: T[], value: T, on: boolean): T[] {
  const without = list.filter((x) => x !== value);
  return on ? [...without, value] : without;
}
