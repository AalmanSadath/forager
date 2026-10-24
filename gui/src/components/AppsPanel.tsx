import { useMemo, useState } from 'react';
import { useAppStore } from '../store/appStore';
import { countUpdates, formatsInUse, groupApps, type Chip } from '../appsView';
import AppCard from './AppCard';
import { BrandMark, FORMAT_NAMES } from './shared/icons';

const SECTION_CHIPS: { chip: Chip; label: string }[] = [
  { chip: 'all', label: 'All' },
  { chip: 'updates', label: 'Updates' },
  { chip: 'installed', label: 'Installed' },
  { chip: 'not_installed', label: 'Not installed' },
  { chip: 'problems', label: 'Problems' },
];

/** The apps grid: the page Forager opens on, like BifroSSH's Hosts. */
export default function AppsPanel() {
  const apps = useAppStore((s) => s.apps);
  const appsError = useAppStore((s) => s.appsError);
  const checks = useAppStore((s) => s.checks);
  const checkingAll = useAppStore((s) => s.checkingAll);
  const checkApps = useAppStore((s) => s.checkApps);
  const setPage = useAppStore((s) => s.setPage);
  const [chip, setChip] = useState<Chip>('all');
  const [search, setSearch] = useState('');

  const sections = useMemo(() => groupApps(apps, checks, chip, search), [apps, checks, chip, search]);
  const formats = useMemo(() => formatsInUse(apps, checks), [apps, checks]);
  const updates = countUpdates(apps, checks);

  return (
    <div className="panel apps-panel">
      <div className="panel-title-row">
        <h1 className="panel-title">Apps</h1>
      </div>
      <div className="panel-toolbar">
        <button className="btn-primary" onClick={() => setPage({ kind: 'add' })}>+ Add App</button>
        <button className="btn-secondary" onClick={() => checkApps()} disabled={checkingAll || apps.length === 0}>
          {checkingAll ? 'Checking…' : 'Check all'}
        </button>
        {updates > 0 && (
          <button className="btn-secondary" onClick={() => setChip('updates')}>
            Updates ({updates})
          </button>
        )}
        <input
          className="apps-search"
          type="search"
          placeholder="Filter by name, source or format"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>

      {appsError && <p className="form-error">{appsError}</p>}

      {apps.length === 0 ? (
        <div className="apps-empty">
          <BrandMark size={56} />
          <p>No apps tracked yet. Add one by its GitHub, GitLab or Codeberg address.</p>
          <button className="btn-primary" onClick={() => setPage({ kind: 'add' })}>+ Add App</button>
        </div>
      ) : (
        <>
          <div className="chips">
            {SECTION_CHIPS.map(({ chip: c, label }) => (
              <button key={c} className={`chip${chip === c ? ' active' : ''}`} onClick={() => setChip(c)}>
                {label}
              </button>
            ))}
            {formats.map((f) => (
              <button key={f} className={`chip${chip === f ? ' active' : ''}`} onClick={() => setChip(f)}>
                {FORMAT_NAMES[f]}
              </button>
            ))}
          </div>
          {sections.length === 0 && <p className="list-empty">Nothing matches.</p>}
          {sections.map((section) => (
            <section key={section.key} className="apps-section">
              <h2 className="section-title">{section.title}</h2>
              <div className="apps-grid">
                {section.items.map(({ app, model }) => (
                  <AppCard key={app.id} app={app} model={model} onOpen={() => setPage({ kind: 'app', id: app.id })} />
                ))}
              </div>
            </section>
          ))}
        </>
      )}
    </div>
  );
}
