/** How the activity page groups and filters history. Pure, so it is tested. */
import type { HistoryEvent } from './types';

export type ActivityChip = 'all' | 'changes' | 'found' | 'errors' | 'timer';

export const ACTIVITY_CHIPS: { chip: ActivityChip; label: string }[] = [
  { chip: 'all', label: 'All' },
  { chip: 'changes', label: 'Installs and removals' },
  { chip: 'found', label: 'Updates found' },
  { chip: 'errors', label: 'Errors' },
  { chip: 'timer', label: 'Background' },
];

export function matchesChip(e: HistoryEvent, chip: ActivityChip): boolean {
  switch (chip) {
    case 'all':
      return true;
    case 'changes':
      return e.kind === 'installed' || e.kind === 'updated' || e.kind === 'removed';
    case 'found':
      return e.kind === 'updates_found';
    case 'errors':
      return e.kind === 'failed';
    case 'timer':
      return e.via === 'timer';
  }
}

/** Local midnight of the day `secs` falls on, in milliseconds. */
function dayStart(ms: number): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** Events in days, newest first, labelled Today, Yesterday or the date. */
export function groupByDay(events: HistoryEvent[], now: Date = new Date()): { label: string; events: HistoryEvent[] }[] {
  const today = dayStart(now.getTime());
  const yesterday = dayStart(today - 1);
  const groups: { day: number; label: string; events: HistoryEvent[] }[] = [];
  for (const e of [...events].sort((a, b) => b.time - a.time)) {
    const day = dayStart(e.time * 1000);
    let group = groups[groups.length - 1];
    if (!group || group.day !== day) {
      const label =
        day === today
          ? 'Today'
          : day === yesterday
            ? 'Yesterday'
            : new Date(day).toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' });
      group = { day, label, events: [] };
      groups.push(group);
    }
    group.events.push(e);
  }
  return groups.map(({ label, events }) => ({ label, events }));
}

/** One line saying what happened, without the app's name. */
export function describe(e: HistoryEvent): string {
  const versions = e.from && e.to ? `${e.from} → ${e.to}` : (e.to ?? e.from ?? '');
  switch (e.kind) {
    case 'installed':
      return `Installed ${versions}`.trim();
    case 'updated':
      return `Updated ${versions}`.trim();
    case 'removed':
      return e.from ? `Uninstalled ${e.from}` : 'Stopped tracking';
    case 'failed':
      return 'Failed';
    case 'updates_found':
      return `Update found: ${versions}`;
  }
}
