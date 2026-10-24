import { useEffect, useRef, useState } from 'react';
import * as ipc from '../ipc';
import { useAppStore } from '../store/appStore';
import type { Format, PlanView } from '../types';
import Modal from './shared/Modal';
import { FORMAT_NAMES } from './shared/icons';
import Markdown from './shared/Markdown';

type Phase =
  | { kind: 'planning' }
  | { kind: 'confirm' }
  | { kind: 'working'; id: string; format: Format; step: 'download' | 'install' }
  | { kind: 'distro'; id: string; version: string; pkg: string }
  | { kind: 'done' };

type Result = { id: string; name: string; outcome: 'installed' | 'skipped' | 'failed'; detail?: string };

/** One update to run: an app, and with several installs, one of its formats. */
type Unit = { id: string; plan: PlanView | null; error?: string };

/**
 * Updating one app, or several from "Update all": show what each will
 * install and how it is verified, then download, verify and install them one
 * at a time with progress. An app installed in several formats (an rpm and a
 * Flatpak) has one update per format. Mirrors the CLI's `update` prompts.
 */
export default function UpdateModal({ ids }: { ids: string[] }) {
  const close = useAppStore((s) => s.closeUpdate);
  const loadApps = useAppStore((s) => s.loadApps);
  const checkApps = useAppStore((s) => s.checkApps);
  const clearProgress = useAppStore((s) => s.clearProgress);
  const toast = useAppStore((s) => s.toast);
  const progress = useAppStore((s) => s.progress);

  const [phase, setPhase] = useState<Phase>({ kind: 'planning' });
  const [plans, setPlans] = useState<Unit[]>([]);
  const [acceptUnverified, setAcceptUnverified] = useState(false);
  const [results, setResults] = useState<Result[]>([]);
  // Answers the distro question for the app waiting on it.
  const distroAnswer = useRef<((install: boolean) => void) | null>(null);
  const cancelled = useRef(false);
  const single = ids.length === 1;

  useEffect(() => {
    let live = true;
    (async () => {
      const out: Unit[] = [];
      for (const id of ids) {
        try {
          const results = await ipc.planUpdate(id);
          if (results.length === 0) out.push({ id, plan: null });
          for (const r of results) out.push({ id, plan: r.plan, error: r.error ?? undefined });
        } catch (e) {
          out.push({ id, plan: null, error: String(e) });
        }
      }
      if (live) {
        setPlans(out);
        setPhase({ kind: 'confirm' });
      }
    })();
    return () => {
      live = false;
    };
  }, [ids]);

  const runnable = plans.filter((p) => p.plan);
  // Named by format too when an app has more than one update.
  const nameOf = (id: string, format: Format) =>
    runnable.filter((p) => p.id === id).length > 1 ? `${id} (${FORMAT_NAMES[format]})` : id;
  const unverified = runnable.some((p) => p.plan!.unverified);
  const needsRoot = runnable.some((p) => p.plan!.needs_root);

  const run = async () => {
    const done: Result[] = [];
    for (const { id, plan } of runnable) {
      if (cancelled.current) break;
      const format = plan!.format;
      const name = nameOf(id, format);
      try {
        setPhase({ kind: 'working', id, format, step: 'download' });
        const downloaded = await ipc.startUpdate(id, format);
        if (downloaded.repo_version && downloaded.first_install) {
          // Asked about one app at a time; in "Update all" it is skipped
          // rather than holding the queue on a question.
          const install = single
            ? await new Promise<boolean>((resolve) => {
                distroAnswer.current = resolve;
                setPhase({ kind: 'distro', id, version: downloaded.repo_version!, pkg: downloaded.package ?? id });
              })
            : false;
          if (!install) {
            await ipc.cancelUpdate(id, format);
            done.push({ id, name, outcome: 'skipped', detail: `also in the distro's repositories (${downloaded.repo_version})` });
            continue;
          }
        }
        setPhase({ kind: 'working', id, format, step: 'install' });
        await ipc.finishUpdate(id, format);
        done.push({ id, name, outcome: 'installed', detail: plan!.to });
      } catch (e) {
        done.push({ id, name, outcome: cancelled.current ? 'skipped' : 'failed', detail: cancelled.current ? 'cancelled' : String(e) });
      } finally {
        clearProgress(id);
      }
    }
    await loadApps();
    checkApps(done.filter((r) => r.outcome === 'installed').map((r) => r.id));
    const failed = done.filter((r) => r.outcome === 'failed');
    if (single && failed.length === 0 && done.length === 1) {
      const r = done[0];
      if (r?.outcome === 'installed') toast('success', `Installed ${r.name} ${r.detail}.`);
      close();
      return;
    }
    setResults(done);
    setPhase({ kind: 'done' });
  };

  const cancel = () => {
    if (phase.kind === 'working' && phase.step === 'download') {
      cancelled.current = true;
      ipc.cancelUpdate(phase.id, phase.format);
    } else if (phase.kind === 'distro') {
      distroAnswer.current?.(false);
    } else if (phase.kind !== 'working') {
      for (const { id, plan } of runnable) ipc.cancelUpdate(id, plan!.format);
      close();
    }
  };

  // Installing something new is not an update, and the title should say so.
  const firstInstall = single && plans[0]?.plan && !plans[0].plan.from;
  const title = single ? `${firstInstall ? 'Install' : 'Update'} ${ids[0]}` : `Update ${ids.length} apps`;
  const blocking = phase.kind === 'working';

  return (
    <Modal title={title} className="update-modal" onClose={blocking ? undefined : cancel}>
      {phase.kind === 'planning' && <p className="list-empty">Working out what to install…</p>}

      {phase.kind === 'confirm' && (
        <>
          {plans.map(({ id, plan, error }, index) => (
            <div key={`${id}/${plan?.format ?? index}`} className="plan">
              {(!single || plans.length > 1) && <div className="plan-title">{plan ? nameOf(id, plan.format) : id}</div>}
              {error ? (
                <p className="form-hint form-hint-error">{error}</p>
              ) : !plan ? (
                <p className="form-hint">Already up to date.</p>
              ) : (
                <>
                  <p className="plan-versions mono">
                    {plan.missing ? `${plan.from} (missing)` : (plan.from ?? 'not installed')} → {plan.to} · {FORMAT_NAMES[plan.format]}
                  </p>
                  <dl className="plan-lines">
                    {plan.lines
                      .filter((l) => single || l.tone !== 'normal')
                      .map((l, i) => (
                        <div key={i} className={`plan-line plan-${l.tone}`}>
                          <dt>{l.label}</dt>
                          <dd>{l.value}</dd>
                        </div>
                      ))}
                  </dl>
                  {plan.notes && (
                    <details className="release-notes" open={single && plans.length === 1}>
                      <summary>What's new in {plan.to}</summary>
                      <Markdown text={plan.notes} />
                    </details>
                  )}
                </>
              )}
            </div>
          ))}
          {needsRoot && (
            <p className="form-hint">System packages ask for your password while they install.</p>
          )}
          {unverified && (
            <label className="checkbox-row unverified-row">
              <input type="checkbox" checked={acceptUnverified} onChange={(e) => setAcceptUnverified(e.target.checked)} />
              <span>Install even though nothing can verify {single ? 'this download' : 'some downloads'}</span>
            </label>
          )}
          <div className="modal-actions">
            <button className="btn-secondary" onClick={cancel}>Cancel</button>
            <button className="btn-primary" disabled={runnable.length === 0 || (unverified && !acceptUnverified)} onClick={run}>
              {runnable.length === 1 ? 'Install' : `Install ${runnable.length}`}
            </button>
          </div>
        </>
      )}

      {phase.kind === 'working' && (
        <>
          <p className="update-step">
            {phase.step === 'download' ? 'Downloading and verifying' : 'Installing'} <strong>{nameOf(phase.id, phase.format)}</strong>
            {runnable.length > 1 &&
              ` (${runnable.findIndex((p) => p.id === phase.id && p.plan!.format === phase.format) + 1} of ${runnable.length})`}
          </p>
          <ProgressBar fraction={progress[phase.id] ?? null} />
          {phase.step === 'install' && runnable.find((p) => p.id === phase.id && p.plan!.format === phase.format)?.plan?.needs_root && (
            <p className="form-hint">If a password prompt appears, it is for this install.</p>
          )}
          <div className="modal-actions">
            <button className="btn-secondary" onClick={cancel} disabled={phase.step === 'install'}>
              {phase.step === 'install' ? 'Cannot stop an install' : 'Cancel'}
            </button>
          </div>
        </>
      )}

      {phase.kind === 'distro' && (
        <>
          <p>
            The distribution's repositories also have <span className="mono">{phase.pkg} {phase.version}</span>. System updates
            and this build may replace each other.
          </p>
          <div className="modal-actions">
            <button className="btn-secondary" onClick={() => distroAnswer.current?.(false)}>Don't install</button>
            <button className="btn-primary" onClick={() => distroAnswer.current?.(true)}>Install anyway</button>
          </div>
        </>
      )}

      {phase.kind === 'done' && (
        <>
          <ul className="update-results">
            {results.map((r) => (
              <li key={r.name} className={`update-result update-${r.outcome}`}>
                <strong>{r.name}</strong>
                <span>
                  {r.outcome === 'installed' ? `installed ${r.detail}` : r.outcome === 'skipped' ? `skipped: ${r.detail}` : r.detail}
                </span>
              </li>
            ))}
          </ul>
          <div className="modal-actions">
            <button className="btn-primary" onClick={close}>Close</button>
          </div>
        </>
      )}
    </Modal>
  );
}

function ProgressBar({ fraction }: { fraction: number | null }) {
  return (
    <div className={`progress${fraction === null ? ' progress-unknown' : ''}`} role="progressbar" aria-valuenow={fraction === null ? undefined : Math.round(fraction * 100)}>
      <div className="progress-fill" style={fraction === null ? undefined : { width: `${Math.round(fraction * 100)}%` }} />
    </div>
  );
}
