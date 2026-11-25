import { useEffect, useState } from 'react';
import * as ipc from '../ipc';
import { useAppStore } from '../store/appStore';
import type { EntrySummary } from '../types';

/**
 * The menu entries in the user's applications folder: ones made here for
 * apps nothing else puts in the menu (MATLAB, vendor installers, builds
 * unpacked by hand), ones made by hand or by other tools, and Forager's own
 * for the apps it tracks, which their app pages manage.
 */
export default function DesktopEntriesPanel() {
  const setPage = useAppStore((s) => s.setPage);
  const [entries, setEntries] = useState<EntrySummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState('');

  useEffect(() => {
    let live = true;
    ipc
      .listEntries()
      .then((list) => live && setEntries(list))
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, []);

  const needle = filter.trim().toLowerCase();
  const shown = (entries ?? []).filter(
    (e) => !needle || [e.name, e.id, e.exec ?? ''].some((v) => v.toLowerCase().includes(needle)),
  );

  return (
    <div className="panel apps-panel">
      <div className="panel-toolbar">
        <button className="btn-primary" onClick={() => setPage({ kind: 'entry' })}>+ New entry</button>
        <input
          className="apps-search"
          placeholder="Filter by name, file or program"
          aria-label="Filter entries"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
      </div>
      <p className="form-hint entries-intro">
        Menu entries for apps installed outside any package, such as MATLAB or a vendor&rsquo;s installer. They are saved
        in <span className="mono">~/.local/share/applications</span>, where every app launcher looks.
      </p>
      {error && <p className="form-error">{error}</p>}
      {entries === null && !error && <p className="list-empty">Reading your menu entries…</p>}
      {entries?.length === 0 && (
        <p className="list-empty">No menu entries of your own yet. New entry makes one for an app the menu is missing.</p>
      )}
      {entries && entries.length > 0 && shown.length === 0 && <p className="list-empty">Nothing matches the filter.</p>}
      <div className="apps-grid">
        {shown.map((entry) => (
          <EntryCard key={entry.id} entry={entry} onOpen={() => setPage({ kind: 'entry', id: entry.id })} />
        ))}
      </div>
    </div>
  );
}

function EntryCard({ entry, onOpen }: { entry: EntrySummary; onOpen: () => void }) {
  const marks = [
    entry.owner && 'tracked app',
    entry.missing_program && 'program missing',
    entry.hidden && 'hidden',
  ].filter(Boolean);
  return (
    <div
      className="app-card"
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onOpen();
        }
      }}
      title={entry.file}
      aria-label={[entry.name, `${entry.id}.desktop`, ...marks].join(', ')}
    >
      <div className="app-card-icon">
        <EntryIcon url={entry.icon_url} name={entry.name} />
      </div>
      <div className="app-card-info">
        <div className="app-card-name-row">
          <span className="card-title">{entry.name}</span>
          <span className="app-card-tags">
            {entry.owner && <span className="tag" title={`Forager's entry for ${entry.owner}; its app page manages it`}>Forager</span>}
            {entry.missing_program && <span className="tag tag-warn">program missing</span>}
            {entry.hidden && <span className="tag">hidden</span>}
          </span>
        </div>
        <div className="card-sub mono">{entry.id}.desktop</div>
      </div>
    </div>
  );
}

/** An entry's icon, or its first letter when it has none that can be shown. */
export function EntryIcon({ url, name, size = 32 }: { url: string | null; name: string; size?: number }) {
  if (url) return <img src={url} width={size} height={size} alt="" />;
  return (
    <span className="entry-icon-letter" style={{ width: size, height: size, fontSize: size * 0.5 }} aria-hidden>
      {(name.trim()[0] ?? '?').toUpperCase()}
    </span>
  );
}
