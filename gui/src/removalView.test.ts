import { describe, expect, it } from 'vitest';
import { removalChoices, toggled } from './removalView';
import type { Cleanup, Format, InstalledView } from './types';

const install = (format: Format): InstalledView => ({
  tag: 'v1.0',
  format,
  asset: '',
  package: null,
  path: null,
  links: [],
  menu_entry: 'unavailable',
});

const cleanup: Cleanup = {
  leftovers: [{ path: '/home/u/.config/tool', bytes: 10 }],
  remotes: [{ name: 'tool-origin', url: 'https://example.org/repo', system: false }],
};

describe('removal choices', () => {
  it('uninstalls every install unless kept', () => {
    const installs = [install('rpm'), install('flatpak')];
    expect(removalChoices(installs, [], cleanup, [], []).uninstall).toEqual(['rpm', 'flatpak']);
    expect(removalChoices(installs, ['rpm'], cleanup, [], []).uninstall).toEqual(['flatpak']);
  });

  it('offers repositories only while the Flatpak goes', () => {
    const installs = [install('appimage'), install('flatpak')];
    expect(removalChoices(installs, [], cleanup, [], []).remotes).toHaveLength(1);
    expect(removalChoices(installs, ['flatpak'], cleanup, [], []).remotes).toEqual([]);
    expect(removalChoices([install('appimage')], [], cleanup, [], []).remotes).toEqual([]);
  });

  it('drops a ticked repository with the Flatpak and brings it back with it', () => {
    const installs = [install('flatpak')];
    const ticked = ['tool-origin'];
    expect(removalChoices(installs, [], cleanup, ticked, []).repos).toEqual(['tool-origin']);
    expect(removalChoices(installs, ['flatpak'], cleanup, ticked, []).repos).toEqual([]);
    expect(removalChoices(installs, [], cleanup, ticked, []).repos).toEqual(['tool-origin']);
  });

  it('only deletes leftovers that were offered', () => {
    const purge = ['/home/u/.config/tool', '/home/u/.ssh'];
    expect(removalChoices([install('appimage')], [], cleanup, [], purge).purge).toEqual(['/home/u/.config/tool']);
  });

  it('warns about a password only for system packages', () => {
    expect(removalChoices([install('deb')], [], cleanup, [], []).systemHint).toMatch(/password/);
    expect(removalChoices([install('pacman'), install('flatpak')], [], cleanup, [], []).systemHint).toMatch(/password/);
    expect(removalChoices([install('deb')], ['deb'], cleanup, [], []).systemHint).toBeUndefined();
    expect(removalChoices([install('flatpak')], [], cleanup, [], []).systemHint).toBeUndefined();
  });
});

describe('toggled', () => {
  it('adds and removes without duplicates', () => {
    expect(toggled(['a'], 'b', true)).toEqual(['a', 'b']);
    expect(toggled(['a', 'b'], 'b', true)).toEqual(['a', 'b']);
    expect(toggled(['a', 'b'], 'a', false)).toEqual(['b']);
    expect(toggled([], 'a', false)).toEqual([]);
  });
});
