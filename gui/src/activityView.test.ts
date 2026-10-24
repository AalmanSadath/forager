import { describe as suite, expect, it } from 'vitest';
import { describe, groupByDay, matchesChip } from './activityView';
import type { HistoryEvent } from './types';

const at = (iso: string) => new Date(iso).getTime() / 1000;
const ev = (time: string, extra: Partial<HistoryEvent> = {}): HistoryEvent => ({
  time: at(time),
  app: 'tool',
  kind: 'updated',
  via: 'gui',
  ...extra,
});

suite('activity', () => {
  it('groups by local day, newest first', () => {
    const now = new Date('2026-10-24T15:00:00');
    const groups = groupByDay(
      [ev('2026-10-22T09:00:00'), ev('2026-10-24T08:00:00'), ev('2026-10-23T23:59:00'), ev('2026-10-24T12:00:00')],
      now,
    );
    expect(groups.map((g) => g.label.startsWith('Thursday') ? 'Thursday' : g.label)).toEqual(['Today', 'Yesterday', 'Thursday']);
    expect(groups[0].events.map((e) => new Date(e.time * 1000).getHours())).toEqual([12, 8]);
  });

  it('filters by chip', () => {
    const failed = ev('2026-10-24T08:00:00', { kind: 'failed', via: 'timer' });
    const found = ev('2026-10-24T08:00:00', { kind: 'updates_found' });
    expect(matchesChip(failed, 'errors')).toBe(true);
    expect(matchesChip(failed, 'timer')).toBe(true);
    expect(matchesChip(failed, 'changes')).toBe(false);
    expect(matchesChip(found, 'found')).toBe(true);
  });

  it('describes what happened', () => {
    expect(describe(ev('2026-10-24T08:00:00', { from: '1.0', to: '2.0' }))).toBe('Updated 1.0 → 2.0');
    expect(describe(ev('2026-10-24T08:00:00', { kind: 'installed', to: '2.0' }))).toBe('Installed 2.0');
    expect(describe(ev('2026-10-24T08:00:00', { kind: 'removed' }))).toBe('Stopped tracking');
    expect(describe(ev('2026-10-24T08:00:00', { kind: 'removed', from: '2.0' }))).toBe('Uninstalled 2.0');
  });
});
