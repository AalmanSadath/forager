import { useEffect, useState } from 'react';
import * as ipc from '../../ipc';
import type { DoctorCheck } from '../../types';

const LEVEL_LABEL = { ok: 'OK', warn: 'Warning', fail: 'Problem' } as const;

/**
 * What Forager needs from the system, as `forager doctor` checks it: the
 * package manager, PackageKit, PATH, signature tools, the GitHub token and
 * the background timer, each with a fix for anything missing.
 */
export default function SystemSection() {
  const [checks, setChecks] = useState<DoctorCheck[] | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setRunning(true);
    setError(null);
    try {
      setChecks(await ipc.runDoctor());
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  };

  useEffect(() => {
    run();
  }, []);

  const problems = checks?.filter((c) => c.level !== 'ok').length ?? 0;
  return (
    <section className="panel-section">
      <h3>System check</h3>
      <div className="setting-row">
        <div className="setting-label">
          {checks === null
            ? 'Checking…'
            : problems === 0
              ? 'Everything Forager needs is in place.'
              : `${problems} thing${problems === 1 ? '' : 's'} to look at.`}
          <p className="form-hint">
            The same checks as <span className="mono">forager doctor</span>.
          </p>
        </div>
        <button className="btn-secondary" onClick={run} disabled={running}>
          {running ? 'Checking…' : 'Check again'}
        </button>
      </div>
      {checks && (
        <ul className="doctor-list">
          {checks.map((c) => (
            <li key={c.name} className={`doctor-check doctor-${c.level}`}>
              <span className="doctor-level">{LEVEL_LABEL[c.level]}</span>
              <div>
                <strong>{c.name}</strong>: {c.detail}
                {c.fix && <p className="form-hint">Fix: {c.fix}</p>}
              </div>
            </li>
          ))}
        </ul>
      )}
      {error && <p className="form-error">{error}</p>}
    </section>
  );
}
