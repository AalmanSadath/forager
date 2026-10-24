import { filterCounts } from '../../assetsView';
import type { Format, Preview } from '../../types';
import { FORMAT_NAMES } from '../shared/icons';
import type { SettingsDraft } from './form';

const FORMATS: Format[] = ['rpm', 'deb', 'pacman', 'flatpak', 'appimage', 'tarball'];

/** How the app is installed: format, filter, executables, prereleases. */
export default function SettingsSection({
  draft,
  onChange,
  preview,
  idField,
}: {
  draft: SettingsDraft;
  onChange: (d: SettingsDraft) => void;
  preview: Preview | null;
  /** The id input on the add page; read-only text on an app's page. */
  idField: React.ReactNode;
}) {
  const set = (patch: Partial<SettingsDraft>) => onChange({ ...draft, ...patch });
  const counts = preview ? filterCounts(preview.candidates) : null;
  const chosen = preview?.candidates.find((c) => c.verdict.kind === 'chosen');
  const tarball = draft.format === 'tarball' || (!draft.format && chosen?.format === 'tarball');

  return (
    <section className="page-card">
      <h3>Settings</h3>
      {idField}
      <div className="form-group">
        <label htmlFor="format">Format</label>
        <select id="format" value={draft.format} onChange={(e) => set({ format: e.target.value as Format | '' })}>
          <option value="">Automatic{chosen?.format ? ` (${FORMAT_NAMES[chosen.format]})` : ''}</option>
          {FORMATS.map((f) => (
            <option key={f} value={f}>{FORMAT_NAMES[f]}</option>
          ))}
        </select>
        <p className="form-hint">
          Automatic prefers this system's own package format, then Flatpak, AppImage and tarball.
        </p>
      </div>
      <div className="form-group">
        <label htmlFor="filter">Asset filter</label>
        <input
          id="filter"
          className={`mono${preview?.filter_error ? ' input-invalid' : ''}`}
          placeholder="Regular expression, e.g. musl|static"
          value={draft.filter}
          onChange={(e) => set({ filter: e.target.value })}
          spellCheck={false}
        />
        {preview?.filter_error ? (
          <p className="form-hint form-hint-error">{preview.filter_error}</p>
        ) : (
          <p className="form-hint">
            Only assets whose name matches are considered.
            {draft.filter.trim() && counts && ` ${counts.passing} of ${counts.packages} packages match.`}
          </p>
        )}
      </div>
      {tarball && (
        <div className="form-group">
          <label htmlFor="bins">Executables</label>
          <input
            id="bins"
            className="mono"
            placeholder="Found automatically, e.g. bin/tool"
            value={draft.bins}
            onChange={(e) => set({ bins: e.target.value })}
            spellCheck={false}
          />
          <p className="form-hint">
            Paths inside the archive to link into ~/.local/bin, separated by commas.
          </p>
        </div>
      )}
      <label className="checkbox-row">
        <input type="checkbox" checked={draft.prerelease} onChange={(e) => set({ prerelease: e.target.checked })} />
        <span>Include prereleases</span>
      </label>
    </section>
  );
}
