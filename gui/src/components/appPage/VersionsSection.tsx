import { useEffect, useState } from 'react';
import * as ipc from '../../ipc';
import { useAppStore } from '../../store/appStore';
import type { AppView, ReleaseView } from '../../types';

const SHOWN = 8;

/**
 * Every release the app follows, to install another one: usually rolling
 * back after a bad update, which also skips the release rolled back from.
 */
export default function VersionsSection({ app }: { app: AppView }) {
  const openUpdate = useAppStore((s) => s.openUpdate);
  const [releases, setReleases] = useState<ReleaseView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [all, setAll] = useState(false);

  useEffect(() => {
    let live = true;
    ipc
      .listReleases(app.id)
      .then((r) => live && setReleases(r))
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [app.id, app.tag_filter, app.prerelease]);

  const installed = new Set(app.installs.map((i) => i.tag));
  const shown = all ? (releases ?? []) : (releases ?? []).slice(0, SHOWN);

  return (
    <section className="page-card">
      <h3>Versions</h3>
      {error && <p className="form-error">{error}</p>}
      {!releases && !error && <p className="list-empty">Loading…</p>}
      {releases && (
        <ul className="version-list">
          {shown.map((r) => (
            <li key={r.tag} className="version-row">
              <span className="mono">{r.tag}</span>
              <span className="version-date">
                {r.published_at && new Date(r.published_at).toLocaleDateString()}
                {r.prerelease && ' · prerelease'}
                {app.skipped === r.tag && ' · skipped'}
              </span>
              {installed.has(r.tag) ? (
                <span className="tag">installed</span>
              ) : (
                <button className="btn-secondary btn-sm" onClick={() => openUpdate([app.id], r.tag)}>
                  Install
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
      {releases && releases.length > SHOWN && (
        <button className="link-btn" onClick={() => setAll(!all)}>
          {all ? 'Show fewer' : `Show all ${releases.length}`}
        </button>
      )}
    </section>
  );
}
