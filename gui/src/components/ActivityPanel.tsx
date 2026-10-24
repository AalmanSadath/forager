import { useCallback, useEffect, useMemo, useState } from 'react';
import * as ipc from '../ipc';
import { useAppStore } from '../store/appStore';
import { ACTIVITY_CHIPS, describe, groupByDay, matchesChip, type ActivityChip } from '../activityView';
import type { HistoryEvent } from '../types';
import { FORMAT_NAMES } from './shared/icons';

const VIA: Record<HistoryEvent['via'], string> = { cli: 'CLI', gui: 'App', timer: 'Background' };
const LIMIT = 500;

/**
 * What Forager did, from the window, the CLI and the background check alike,
 * newest first and grouped by day.
 */
export default function ActivityPanel() {
  const apps = useAppStore((s) => s.apps);
  const setPage = useAppStore((s) => s.setPage);
  const [events, setEvents] = useState<HistoryEvent[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [chip, setChip] = useState<ActivityChip>('all');
  const [open, setOpen] = useState<string | null>(null);

  const load = useCallback(() => {
    ipc
      .listHistory(LIMIT)
      .then((e) => {
        setEvents(e);
        setError(null);
      })
      .catch((e) => setError(String(e)));
  }, []);

  // The CLI and the timer write here too, so look again on return.
  useEffect(() => {
    load();
    window.addEventListener('focus', load);
    return () => window.removeEventListener('focus', load);
  }, [load]);

  const groups = useMemo(() => groupByDay((events ?? []).filter((e) => matchesChip(e, chip))), [events, chip]);
  const tracked = new Set(apps.map((a) => a.id));

  return (
    <div className="panel activity-panel">
      <div className="panel-title-row">
        <h1 className="panel-title">Activity</h1>
        <button className="btn-secondary btn-sm" onClick={load} style={{ marginLeft: 'auto' }}>Refresh</button>
      </div>
      <div className="chips activity-chips">
        {ACTIVITY_CHIPS.map(({ chip: c, label }) => (
          <button key={c} className={`chip${chip === c ? ' active' : ''}`} onClick={() => setChip(c)}>
            {label}
          </button>
        ))}
      </div>
      {error && <p className="form-error">{error}</p>}
      {events && groups.length === 0 && (
        <p className="list-empty">
          {events.length === 0 ? 'Nothing yet. Installs, updates, removals and errors show up here.' : 'Nothing matches.'}
        </p>
      )}
      {groups.map((g) => (
        <section key={g.label} className="activity-day">
          <h2 className="section-title">{g.label}</h2>
          <ul className="activity-list">
            {g.events.map((e) => {
              const key = `${e.time}-${e.app}-${e.kind}`;
              const expandable = !!e.message;
              return (
                <li key={key} className={`activity-row activity-${e.kind}`}>
                  <span className="activity-time mono">
                    {new Date(e.time * 1000).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}
                  </span>
                  <span className={`activity-mark mark-${e.kind}`} aria-hidden />
                  {tracked.has(e.app) ? (
                    <button className="link-btn activity-app" onClick={() => setPage({ kind: 'app', id: e.app })}>
                      {e.app}
                    </button>
                  ) : (
                    <span className="activity-app">{e.app}</span>
                  )}
                  <span className="activity-what">{describe(e)}</span>
                  {e.format && <span className="tag">{FORMAT_NAMES[e.format]}</span>}
                  <span className={`tag via-${e.via}`}>{VIA[e.via]}</span>
                  {expandable && (
                    <button className="link-btn activity-more" onClick={() => setOpen(open === key ? null : key)}>
                      {open === key ? 'Hide' : 'Details'}
                    </button>
                  )}
                  {expandable && open === key && <p className="activity-message mono">{e.message}</p>}
                </li>
              );
            })}
          </ul>
        </section>
      ))}
    </div>
  );
}
