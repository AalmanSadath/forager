import { useEffect, useMemo, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import * as ipc from '../ipc';
import { useAppStore } from '../store/appStore';
import { cardModel } from '../appsView';
import type { AppView, Format, InstalledView, UpdateState } from '../types';
import AssetsSection from './appPage/AssetsSection';
import VersionsSection from './appPage/VersionsSection';
import SettingsSection from './appPage/SettingsSection';
import SignatureSection from './appPage/SignatureSection';
import {
  draftOf,
  sameDraft,
  signatureDraftOf,
  signatureForm,
  toInput,
  usePreview,
  type SettingsDraft,
  type SignatureDraft,
} from './appPage/form';
import ConfirmModal from './shared/ConfirmModal';
import { BackIcon, ExternalIcon, FormatIcon, FORGE_NAMES, FORMAT_NAMES } from './shared/icons';

/**
 * One app's own page: where it stands, what the latest release offers, and
 * how it is installed. A full page rather than BifroSSH's side drawer, since
 * managing apps is what Forager is for.
 */
export default function AppPage({ id }: { id: string }) {
  const app = useAppStore((s) => s.apps.find((a) => a.id === id));
  const setPage = useAppStore((s) => s.setPage);
  if (!app) {
    return (
      <div className="panel">
        <p className="list-empty">This app is no longer tracked.</p>
        <button className="btn-secondary" onClick={() => setPage({ kind: 'apps' })}>Back to apps</button>
      </div>
    );
  }
  // Keyed, so the form starts over when the app itself is replaced after a save.
  return <AppPageBody key={JSON.stringify(app)} app={app} />;
}

function AppPageBody({ app }: { app: AppView }) {
  const setPage = useAppStore((s) => s.setPage);
  const loadApps = useAppStore((s) => s.loadApps);
  const checkApps = useAppStore((s) => s.checkApps);
  const toast = useAppStore((s) => s.toast);
  const entry = useAppStore((s) => s.checks[app.id]);
  const openUpdate = useAppStore((s) => s.openUpdate);

  const [draft, setDraft] = useState<SettingsDraft>(() => draftOf(app));
  const [signature, setSignature] = useState<SignatureDraft>(() => signatureDraftOf(app));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [removing, setRemoving] = useState(false);
  // Formats to uninstall when removing: every install unless unticked.
  const [keep, setKeep] = useState<Format[]>([]);
  const uninstall = app.installs.map((i) => i.format).filter((f) => !keep.includes(f));

  const input = useMemo(() => toInput(draft), [draft]);
  const { preview, error: previewError, loading, refresh } = usePreview(app.spec, input, app.id);
  const sigForm = signatureForm(signature, app);
  const dirty = !sameDraft(draft, draftOf(app)) || sigForm.kind !== 'keep';
  const model = cardModel(app, entry);
  const status = entry?.status;

  // Escape goes back, as it closes a drawer in BifroSSH.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !removing) setPage({ kind: 'apps' });
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [removing, setPage]);

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      await ipc.saveApp(app.id, input, sigForm);
      await loadApps();
      checkApps([app.id]);
      toast('success', `Saved ${app.id}.`);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const setSkipped = async (tag: string | null) => {
    try {
      await (tag ? ipc.skipRelease(app.id, tag) : ipc.unskipRelease(app.id));
      await loadApps();
      checkApps([app.id]);
    } catch (e) {
      toast('error', String(e));
    }
  };

  const remove = async () => {
    setRemoving(false);
    try {
      await ipc.removeApp(app.id, uninstall);
      setPage({ kind: 'apps' });
      await loadApps();
      toast('success', uninstall.length > 0 ? `Removed and uninstalled ${app.id}.` : `Stopped tracking ${app.id}.`);
    } catch (e) {
      toast('error', String(e));
    }
  };

  return (
    <div className="panel app-page">
      <header className="page-header">
        <button className="icon-btn" onClick={() => setPage({ kind: 'apps' })} title="Back to apps (Esc)">
          <BackIcon />
        </button>
        <div className="page-header-icon">
          <FormatIcon format={model.format} size={26} />
        </div>
        <div className="page-header-title">
          <h1>{app.id}</h1>
          <button className="link-btn" onClick={() => openUrl(app.web_url).catch(() => {})}>
            {app.source} <ExternalIcon />
          </button>
        </div>
        <div className="page-header-status" title={model.label}>
          <span className={`dot dot-${model.dot}`} />
          {/* The full error is under Status; the pill only names the state. */}
          <span>{entry?.error ? 'Check failed' : model.label}</span>
        </div>
        <div className="page-header-actions">
          <button className="btn-secondary" onClick={() => checkApps([app.id])} disabled={entry?.checking}>
            {entry?.checking ? 'Checking…' : 'Check'}
          </button>
          {model.action && !dirty && (
            <button className="btn-primary" onClick={() => openUpdate([app.id])}>
              {model.action === 'install'
                ? 'Install'
                : model.action === 'reinstall'
                  ? 'Reinstall'
                  : model.action === 'switch'
                    ? `Switch to ${draft.format ? FORMAT_NAMES[draft.format] : 'new format'}`
                    : `Update to ${status?.latest_tag ?? 'latest'}`}
            </button>
          )}
          {model.action === 'update' && !dirty && status && (
            <button className="btn-secondary" onClick={() => setSkipped(status.latest_tag)} title="Do not offer this release; newer ones are">
              Skip {status.latest_tag}
            </button>
          )}
          {dirty && (
            <button className="btn-primary" onClick={save} disabled={saving || !!preview?.filter_error}>
              {saving ? 'Saving…' : 'Save'}
            </button>
          )}
        </div>
      </header>

      {error && <p className="form-error">{error}</p>}

      <div className="page-columns">
        <div className="page-column">
          <section className="page-card">
            <h3>Status</h3>
            <dl className="facts">
              {app.installs.length === 0 && (
                <>
                  <dt>Installed</dt>
                  <dd>Not installed</dd>
                </>
              )}
              {app.installs.map((installed) => (
                <InstallFacts
                  key={installed.format}
                  installed={installed}
                  several={app.installs.length > 1}
                  state={status?.installs?.find((i) => i.format === installed.format)?.state}
                />
              ))}
              {app.skipped && status?.skipped && (
                <>
                  <dt>Skipped</dt>
                  <dd>
                    <span className="mono">{app.skipped}</span> ·{' '}
                    <button className="link-btn" onClick={() => setSkipped(null)}>
                      Offer it again
                    </button>
                  </dd>
                </>
              )}
              <dt>Latest</dt>
              <dd>
                {status ? <span className="mono">{status.latest_tag}</span> : entry?.error ? <span className="text-danger">{entry.error}</span> : 'Not checked yet'}
              </dd>
              {status?.repo_version && (
                <>
                  <dt>Distro</dt>
                  <dd>
                    The distribution's repositories have <span className="mono">{status.repo_version}</span>; system
                    updates and this build may replace each other.
                  </dd>
                </>
              )}
              <dt>Source</dt>
              <dd>{FORGE_NAMES[app.forge]}</dd>
              {status && (
                <>
                  <dt>Checked</dt>
                  <dd>{new Date(status.checked_at * 1000).toLocaleTimeString()}</dd>
                </>
              )}
            </dl>
          </section>
          <SettingsSection
            webPage={app.forge === 'html'}
            draft={draft}
            onChange={setDraft}
            preview={preview}
            idField={
              <div className="form-group">
                <label htmlFor="app-id">Id</label>
                <input id="app-id" className="mono" value={app.id} readOnly />
              </div>
            }
          />
          <SignatureSection draft={signature} onChange={setSignature} app={app} forge={app.forge} />
        </div>
        <div className="page-column">
          <AssetsSection
            preview={preview}
            pinned={!!draft.format}
            installed={app.installs.length > 0}
            loading={loading}
            error={previewError}
            onRefresh={refresh}
          />
          <VersionsSection app={app} />
          <section className="page-card danger-zone">
            <div>
              <h3>Remove</h3>
              <p className="form-hint">Stop tracking this app, and uninstall it if you like.</p>
            </div>
            <button className="btn-danger" onClick={() => setRemoving(true)}>Remove app</button>
          </section>
        </div>
      </div>

      {removing && (
        <ConfirmModal
          question={`Remove ${app.id}?`}
          hint={uninstall.some((f) => ['rpm', 'deb', 'pacman'].includes(f)) ? 'Uninstalling a system package asks for your password.' : undefined}
          confirmLabel="Remove"
          onConfirm={remove}
          onCancel={() => setRemoving(false)}
        >
          {app.installs.map((installed) => (
            <label key={installed.format} className="checkbox-row">
              <input
                type="checkbox"
                checked={!keep.includes(installed.format)}
                onChange={(e) =>
                  setKeep((k) => (e.target.checked ? k.filter((f) => f !== installed.format) : [...k, installed.format]))
                }
              />
              <span>
                Also uninstall {app.installs.length > 1 ? `the ${FORMAT_NAMES[installed.format]} ` : ''}
                {installed.tag}
              </span>
            </label>
          ))}
        </ConfirmModal>
      )}
    </div>
  );
}

const INSTALL_STATES: Partial<Record<UpdateState, string>> = {
  outdated: 'update available',
  missing: 'removed outside Forager',
  up_to_date: 'up to date',
};

/** One install in the Status card; with several, each is named by its format. */
function InstallFacts({ installed, several, state }: { installed: InstalledView; several: boolean; state?: UpdateState }) {
  return (
    <>
      <dt>{several ? FORMAT_NAMES[installed.format] : 'Installed'}</dt>
      <dd>
        <span className="mono">{installed.tag}</span>
        {several ? (state && INSTALL_STATES[state] ? ` · ${INSTALL_STATES[state]}` : '') : ` · ${FORMAT_NAMES[installed.format]}`}
      </dd>
      {installed.package && (
        <>
          <dt>{several ? '' : 'Package'}</dt>
          <dd className="mono">{installed.package}</dd>
        </>
      )}
      {installed.path && (
        <>
          <dt>{several ? '' : 'Location'}</dt>
          <dd className="mono">{installed.path}</dd>
        </>
      )}
      {installed.links.length > 0 && (
        <>
          <dt>{several ? '' : 'Commands'}</dt>
          <dd className="mono">{installed.links.join(', ')}</dd>
        </>
      )}
    </>
  );
}
