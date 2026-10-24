/**
 * How the apps grid sorts, labels and filters apps. Pure, so it is tested
 * without a window.
 */
import type { AppView, Format, StatusView } from './types';

/** What the frontend knows about one app's latest check. */
export interface CheckEntry {
  checking: boolean;
  status: StatusView | null;
  error: string | null;
}

export type Section = 'updates' | 'problems' | 'installed' | 'not_installed';
export type Dot = 'on' | 'off' | 'error' | 'update' | 'busy';

export const SECTIONS: { key: Section; title: string }[] = [
  { key: 'updates', title: 'Updates available' },
  { key: 'problems', title: 'Problems' },
  { key: 'installed', title: 'Installed' },
  { key: 'not_installed', title: 'Not installed' },
];

export interface CardModel {
  section: Section;
  dot: Dot;
  /** Monospace line under the name. */
  versions: string;
  /** Short status for screen readers and the app page. */
  label: string;
  /** The format shown on the card: installed, else what would be installed. */
  format: Format | null;
  /** Every installed format, the preferred one first; `[format]` when none is installed. */
  formats: Format[];
  /** The primary action, if one makes sense from the card. */
  action: 'update' | 'install' | 'reinstall' | 'switch' | null;
}

export function cardModel(app: AppView, entry: CheckEntry | undefined): CardModel {
  return { ...baseModel(app, entry), formats: installedFormats(app, entry) };
}

function installedFormats(app: AppView, entry: CheckEntry | undefined): Format[] {
  if (app.installs.length > 0) return app.installs.map((i) => i.format);
  const format = entry?.status?.asset_format ?? app.format;
  return format ? [format] : [];
}

function baseModel(app: AppView, entry: CheckEntry | undefined): Omit<CardModel, 'formats'> {
  // With several installs, the one needing attention is the one shown.
  const attention = entry?.status?.installs?.find((i) => i.state !== 'up_to_date');
  const installed = attention ? { tag: attention.tag, format: attention.format } : app.installed;
  const status = entry?.status ?? null;
  const format = installed?.format ?? status?.asset_format ?? app.format;
  const busy = entry?.checking ?? false;
  const dot = (d: Dot): Dot => (busy ? 'busy' : d);

  if (entry?.error) {
    // The error itself goes on the card: it is the thing to act on.
    return { section: 'problems', dot: dot('error'), versions: entry.error, label: entry.error, format, action: null };
  }
  if (!status) {
    return installed
      ? { section: 'installed', dot: dot('off'), versions: installed.tag, label: 'Not checked yet', format, action: null }
      : { section: 'not_installed', dot: dot('off'), versions: '—', label: 'Not installed', format, action: 'install' };
  }
  const latest = status.latest_tag;
  switch (status.state) {
    case 'not_installed':
      return { section: 'not_installed', dot: dot('off'), versions: `${latest} available`, label: 'Not installed', format, action: 'install' };
    case 'missing':
      return {
        section: 'problems',
        dot: dot('error'),
        versions: `${installed?.tag ?? '?'} missing`,
        label: 'Removed outside Forager',
        format,
        action: 'reinstall',
      };
    case 'wrong_format':
      return {
        section: 'updates',
        dot: dot('update'),
        versions: `${installed?.tag ?? '?'} → ${latest}`,
        label: `Switch to ${app.format ?? 'another format'}`,
        format,
        action: 'switch',
      };
    case 'outdated':
      return { section: 'updates', dot: dot('update'), versions: `${installed?.tag ?? '?'} → ${latest}`, label: 'Update available', format, action: 'update' };
    case 'up_to_date':
      return { section: 'installed', dot: dot('on'), versions: installed?.tag ?? latest, label: 'Up to date', format, action: null };
  }
}

/** A chip: one of the sections, or a format. */
export type Chip = 'all' | Section | Format;

/** Lower-cased words an app is found by: id, source, formats, package names. */
function haystack(app: AppView, model: CardModel): string {
  return [app.id, app.source, ...model.formats, ...app.installs.map((i) => i.package ?? '')].join(' ').toLowerCase();
}

/** Apps shown for a chip and a search, grouped into sections in order. */
export function groupApps(
  apps: AppView[],
  entries: Record<string, CheckEntry>,
  chip: Chip,
  search: string,
): { key: Section; title: string; items: { app: AppView; model: CardModel }[] }[] {
  const words = search.toLowerCase().split(/\s+/).filter(Boolean);
  const items = apps
    .map((app) => ({ app, model: cardModel(app, entries[app.id]) }))
    .filter(({ model }) => chip === 'all' || model.section === chip || model.formats.includes(chip as Format))
    .filter(({ app, model }) => {
      const hay = haystack(app, model);
      return words.every((w) => hay.includes(w));
    })
    .sort((a, b) => a.app.id.localeCompare(b.app.id));
  return SECTIONS.map(({ key, title }) => ({ key, title, items: items.filter((i) => i.model.section === key) })).filter(
    (s) => s.items.length > 0,
  );
}

/** Formats in use, for the chip row, in a fixed order. */
export function formatsInUse(apps: AppView[], entries: Record<string, CheckEntry>): Format[] {
  const order: Format[] = ['rpm', 'deb', 'pacman', 'flatpak', 'appimage', 'tarball'];
  const used = new Set(apps.flatMap((a) => cardModel(a, entries[a.id]).formats));
  return order.filter((f) => used.has(f));
}

/** Apps with an AppImage or tarball install that is not in the menu. */
export function missingMenuEntries(apps: AppView[]): string[] {
  return apps.filter((a) => a.installs.some((i) => i.menu_entry === 'missing')).map((a) => a.id);
}

export function countUpdates(apps: AppView[], entries: Record<string, CheckEntry>): number {
  return apps.filter((a) => cardModel(a, entries[a.id]).section === 'updates').length;
}

/**
 * Checks saved by an earlier run. Entries written by an older version lack
 * fields this one reads (`installs` came with 0.1.3) and are dropped: they
 * are only a cache, and the next check replaces them.
 */
export function restoreChecks(text: string | null): Record<string, CheckEntry> {
  let saved: unknown;
  try {
    saved = JSON.parse(text ?? '{}');
  } catch {
    return {};
  }
  if (!saved || typeof saved !== 'object') return {};
  const checks: Record<string, CheckEntry> = {};
  for (const [id, entry] of Object.entries(saved as Record<string, Partial<CheckEntry>>)) {
    if (entry?.status && Array.isArray(entry.status.installs)) {
      checks[id] = { status: entry.status, error: null, checking: false };
    }
  }
  return checks;
}
