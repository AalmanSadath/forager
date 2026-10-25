import { useState } from 'react';
import { open, save } from '@tauri-apps/plugin-dialog';
import * as ipc from '../../ipc';
import { useAppStore } from '../../store/appStore';

/**
 * Moving the tracked apps to another machine: an export file of their
 * settings and keys, read back by Import here or `forager import`.
 */
export default function AppsSection() {
  const toast = useAppStore((s) => s.toast);
  const loadApps = useAppStore((s) => s.loadApps);
  const checkApps = useAppStore((s) => s.checkApps);
  const count = useAppStore((s) => s.apps.length);
  const [busy, setBusy] = useState(false);
  const [skipped, setSkipped] = useState<{ id: string; error: string }[]>([]);

  const exportApps = async () => {
    const path = await save({ title: 'Export apps', defaultPath: 'forager-apps.toml' });
    if (!path) return;
    try {
      const n = await ipc.exportApps(path);
      toast('success', `Exported ${n} app${n === 1 ? '' : 's'}.`);
    } catch (e) {
      toast('error', String(e));
    }
  };

  const importApps = async () => {
    const path = await open({ multiple: false, directory: false, title: 'Import apps' });
    if (typeof path !== 'string') return;
    setBusy(true);
    try {
      const results = await ipc.importApps(path);
      const added = results.filter((r) => !r.error).map((r) => r.id);
      setSkipped(results.filter((r) => r.error).map((r) => ({ id: r.id, error: r.error! })));
      await loadApps();
      if (added.length > 0) checkApps(added);
      toast(
        added.length > 0 ? 'success' : 'info',
        added.length > 0
          ? `Tracking ${added.length} more app${added.length === 1 ? '' : 's'}. Install them from the Apps page.`
          : 'Nothing new to track.',
      );
    } catch (e) {
      toast('error', String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="panel-section">
      <h3>Apps</h3>
      <div className="setting-row">
        <div className="setting-label">
          Export
          <p className="form-hint">
            Saves every tracked app's source, settings and signing keys to a file, without what is installed.
          </p>
        </div>
        <button className="btn-secondary" onClick={exportApps} disabled={count === 0}>
          Export…
        </button>
      </div>
      <div className="setting-row">
        <div className="setting-label">
          Import
          <p className="form-hint">
            Tracks the apps in an exported file, other than those tracked already. Ones already installed on this
            machine are found on the next check.
          </p>
        </div>
        <button className="btn-secondary" onClick={importApps} disabled={busy}>
          {busy ? 'Importing…' : 'Import…'}
        </button>
      </div>
      {skipped.length > 0 && (
        <ul className="form-hint">
          {skipped.map((s) => (
            <li key={s.id}>
              <span className="mono">{s.id}</span>: {s.error}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
