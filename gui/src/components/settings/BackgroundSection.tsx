import { useEffect, useState } from 'react';
import * as ipc from '../../ipc';
import type { TimerState } from '../../types';

/**
 * The daily check, a systemd user timer that runs the forager CLI, so it
 * works with the window closed.
 */
export default function BackgroundSection() {
  const [state, setState] = useState<TimerState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    ipc.getTimer().then(setState).catch((e) => setError(String(e)));
  }, []);

  const apply = async (enabled: boolean, autoUpdate: boolean) => {
    setBusy(true);
    setError(null);
    try {
      setState(await ipc.setTimer(enabled, autoUpdate));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="panel-section">
      <h3>Background checks</h3>
      <div className="setting-row">
        <div className="setting-label">
          Check for updates daily
          <p className="form-hint">
            Runs once a day, even with this window closed, and shows a notification when something has an update.
          </p>
        </div>
        <label className="checkbox-row">
          <input
            type="checkbox"
            aria-label="Check for updates daily"
            disabled={!state || busy}
            checked={state?.enabled ?? false}
            onChange={(e) => apply(e.target.checked, state?.auto_update ?? false)}
          />
        </label>
      </div>
      <div className="setting-row">
        <div className="setting-label">
          Install updates that need no password
          <p className="form-hint">
            AppImages, tarballs and Flatpaks are updated automatically. System packages still wait for you, since they
            need a password and run their scripts as root.
          </p>
        </div>
        <label className="checkbox-row">
          <input
            type="checkbox"
            aria-label="Install updates that need no password"
            disabled={!state?.enabled || busy}
            checked={state?.auto_update ?? false}
            onChange={(e) => apply(true, e.target.checked)}
          />
        </label>
      </div>
      {state?.enabled && state.next_run && <p className="form-hint">Next check: {state.next_run}</p>}
      {state && !state.cli && (
        <p className="form-hint form-hint-warn">
          The forager command-line tool was not found next to the app or on PATH; the background check needs it.
        </p>
      )}
      {error && <p className="form-error">{error}</p>}
    </section>
  );
}
