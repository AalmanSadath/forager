import { useState } from 'react';
import { sortCandidates, verdictText, formatSize } from '../../assetsView';
import type { Preview } from '../../types';
import { FORMAT_NAMES } from '../shared/icons';

/** Every asset of the latest release, and what the matcher made of it. */
export default function AssetsSection({
  preview,
  pinned,
  loading,
  error,
  onRefresh,
}: {
  preview: Preview | null;
  pinned: boolean;
  loading: boolean;
  error: string | null;
  onRefresh: () => void;
}) {
  const [showOthers, setShowOthers] = useState(false);
  const sorted = preview ? sortCandidates(preview.candidates) : [];
  // Checksums, signatures and the like: only noise until asked for.
  const isOther = (c: (typeof sorted)[number]) => c.verdict.kind === 'rejected' && c.verdict.reason === 'not_package';
  const packages = sorted.filter((c) => !isOther(c));
  const others = sorted.filter(isOther);
  const shown = showOthers ? [...packages, ...others] : packages;

  return (
    <section className="page-card">
      <div className="page-card-header">
        <h3>Latest release</h3>
        <button className="btn-secondary btn-sm" onClick={onRefresh} disabled={loading}>
          {loading ? 'Loading…' : 'Refresh'}
        </button>
      </div>
      {error && <p className="form-error">{error}</p>}
      {preview && (
        <>
          <p className="release-line">
            <span className="mono">{preview.release.tag}</span>
            {preview.release.name && preview.release.name !== preview.release.tag && <span> · {preview.release.name}</span>}
            {preview.release.published_at && <span> · {new Date(preview.release.published_at).toLocaleDateString()}</span>}
            {preview.release.prerelease && <span className="tag tag-warn">prerelease</span>}
          </p>
          {preview.candidates.length === 0 ? (
            <p className="list-empty">This release has no assets.</p>
          ) : packages.length === 0 && !showOthers ? (
            <p className="list-empty">No packages in this release.</p>
          ) : (
            <ul className="asset-list">
              {shown.map((c) => (
                <li key={c.name} className={`asset-row asset-${c.verdict.kind}`}>
                  <span className="asset-name mono" title={c.name}>{c.name}</span>
                  {c.format && <span className="tag">{FORMAT_NAMES[c.format]}</span>}
                  <span className="asset-size">{formatSize(c.size)}</span>
                  <span className="asset-verdict">{verdictText(c, pinned)}</span>
                </li>
              ))}
            </ul>
          )}
          {others.length > 0 && (
            <button className="link-btn asset-toggle" onClick={() => setShowOthers((v) => !v)}>
              {showOthers ? 'Hide other files' : `Show ${others.length} other file${others.length === 1 ? '' : 's'} (checksums, signatures…)`}
            </button>
          )}
        </>
      )}
      {!preview && !error && <p className="list-empty">Loading the latest release…</p>}
    </section>
  );
}
