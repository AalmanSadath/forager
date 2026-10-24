/** How the app page explains each release asset. Pure, so it is tested. */
import type { CandidateView, Reason } from './types';

const REASONS: Record<Reason, string> = {
  not_package: 'Not a package',
  filter: 'Excluded by the filter',
  debug: 'Debug symbols',
  other_os: 'For another operating system',
  format_not_wanted: 'Format not used here',
  other_arch: 'For another CPU',
  source_archive: 'Source code',
};

export function verdictText(c: CandidateView, pinned: boolean): string {
  switch (c.verdict.kind) {
    case 'chosen':
      return 'Will be installed';
    case 'usable':
      return 'Also installable';
    case 'rejected':
      return c.verdict.reason === 'format_not_wanted' && pinned ? 'Not the chosen format' : REASONS[c.verdict.reason];
  }
}

/** Chosen first, then other installable assets, then the rest in release order. */
export function sortCandidates(list: CandidateView[]): CandidateView[] {
  const rank = (c: CandidateView) => (c.verdict.kind === 'chosen' ? 0 : c.verdict.kind === 'usable' ? 1 : 2);
  return list
    .map((c, i) => ({ c, i }))
    .sort((a, b) => rank(a.c) - rank(b.c) || a.i - b.i)
    .map(({ c }) => c);
}

/** Packages the filter lets through, out of all packages, for its hint. */
export function filterCounts(list: CandidateView[]): { passing: number; packages: number } {
  const packages = list.filter((c) => !(c.verdict.kind === 'rejected' && c.verdict.reason === 'not_package'));
  const passing = packages.filter((c) => !(c.verdict.kind === 'rejected' && c.verdict.reason === 'filter'));
  return { passing: passing.length, packages: packages.length };
}

export function formatSize(bytes: number): string {
  if (bytes <= 0) return '';
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KiB`;
  return `${(bytes / 1048576).toFixed(1)} MiB`;
}
