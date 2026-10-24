import { describe, expect, it } from 'vitest';
import { filterCounts, formatSize, sortCandidates, verdictText } from './assetsView';
import type { CandidateView, Verdict } from './types';

const c = (name: string, verdict: Verdict): CandidateView => ({ name, size: 0, format: null, verdict, updates: null });

describe('assets', () => {
  const list = [
    c('SHA256SUMS', { kind: 'rejected', reason: 'not_package' }),
    c('tool.deb', { kind: 'rejected', reason: 'format_not_wanted' }),
    c('tool.AppImage', { kind: 'usable' }),
    c('tool-arm64.rpm', { kind: 'rejected', reason: 'filter' }),
    c('tool.rpm', { kind: 'chosen' }),
  ];

  it('puts the chosen asset first, then usable ones, then the rest in order', () => {
    expect(sortCandidates(list).map((x) => x.name)).toEqual([
      'tool.rpm',
      'tool.AppImage',
      'SHA256SUMS',
      'tool.deb',
      'tool-arm64.rpm',
    ]);
  });

  it('counts packages the filter lets through, ignoring non-packages', () => {
    expect(filterCounts(list)).toEqual({ passing: 3, packages: 4 });
  });

  it('says a pinned format is the reason, not the system', () => {
    expect(verdictText(list[1], false)).toBe('Format not used here');
    expect(verdictText(list[1], true)).toBe('Not the chosen format');
    expect(verdictText(list[4], false)).toBe('Will be installed');
  });

  it('marks what updates each install of an installed app', () => {
    const flatpak = { ...c('tool.flatpak', { kind: 'usable' }), updates: 'flatpak' as const };
    const withInstall = [...list, flatpak];
    expect(sortCandidates(withInstall)[0].name).toBe('tool.flatpak');
    expect(verdictText(flatpak, false, true)).toBe('Updates the Flatpak install');
    // The pick for a fresh install is just another option once something is installed.
    expect(verdictText(list[4], false, true)).toBe('Also installable');
  });

  it('formats sizes, and hides unknown ones', () => {
    expect(formatSize(0)).toBe('');
    expect(formatSize(300)).toBe('1 KiB');
    expect(formatSize(5 * 1048576)).toBe('5.0 MiB');
  });
});
