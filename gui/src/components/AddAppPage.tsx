import { useEffect, useMemo, useRef, useState } from 'react';
import * as ipc from '../ipc';
import { useAppStore } from '../store/appStore';
import type { SourceInfo } from '../types';
import AssetsSection from './appPage/AssetsSection';
import SettingsSection from './appPage/SettingsSection';
import SignatureSection from './appPage/SignatureSection';
import { EMPTY_DRAFT, signatureForm, toInput, usePreview, type SettingsDraft, type SignatureDraft } from './appPage/form';
import { BackIcon, FORGE_NAMES } from './shared/icons';

/**
 * Adding an app: paste its address, see what its latest release offers and
 * which asset would be installed, adjust, then track it. Same layout as an
 * app's own page.
 */
export default function AddAppPage({ initialUrl }: { initialUrl?: string }) {
  const setPage = useAppStore((s) => s.setPage);
  const loadApps = useAppStore((s) => s.loadApps);
  const checkApps = useAppStore((s) => s.checkApps);
  const toast = useAppStore((s) => s.toast);
  const apps = useAppStore((s) => s.apps);

  const [url, setUrl] = useState(initialUrl ?? '');
  const [unknownHost, setUnknownHost] = useState<string | null>(null);
  const [forge, setForge] = useState<'gitlab' | 'forgejo'>('forgejo');
  const [info, setInfo] = useState<SourceInfo | null>(null);
  const [id, setId] = useState('');
  const [draft, setDraft] = useState<SettingsDraft>(EMPTY_DRAFT);
  const [signature, setSignature] = useState<SignatureDraft>({ kind: 'none', minisignKey: '', gpgFile: '' });
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);

  const input = useMemo(() => toInput(draft), [draft]);
  // The store refuses a second app for the same project, so say so up front.
  const existing = info ? apps.find((a) => a.source === info.display) : undefined;
  const { preview, error: previewError, loading, refresh } = usePreview(info?.spec ?? null, input);

  const lookUp = async (e?: React.FormEvent) => {
    e?.preventDefault();
    setError(null);
    try {
      const parsed = await ipc.parseSource(url.trim(), unknownHost ? forge : null);
      setUnknownHost(parsed.unknown_host);
      setInfo(parsed.info);
      if (parsed.info) setId(parsed.info.default_id);
    } catch (err) {
      setInfo(null);
      setError(String(err));
    }
  };

  // Opened with an address (`forager-gui add <url>`): look it up straight away.
  const lookUpRef = useRef(lookUp);
  lookUpRef.current = lookUp;
  useEffect(() => {
    if (initialUrl) lookUpRef.current();
  }, [initialUrl]);

  const add = async () => {
    if (!info) return;
    setAdding(true);
    setError(null);
    try {
      const app = await ipc.addApp(info.spec, id.trim(), input, signatureForm(signature, null));
      await loadApps();
      checkApps([app.id]);
      setPage({ kind: 'app', id: app.id });
      toast('success', `Tracking ${app.id}.`);
    } catch (err) {
      setError(String(err));
    } finally {
      setAdding(false);
    }
  };

  return (
    <div className="panel app-page">
      <header className="page-header">
        <button className="icon-btn" onClick={() => setPage({ kind: 'apps' })} title="Back to apps">
          <BackIcon />
        </button>
        <div className="page-header-title">
          <h1>Add app</h1>
          {info && <span className="page-header-sub">{FORGE_NAMES[info.forge]} · {info.display}</span>}
        </div>
        <div className="page-header-actions">
          <button className="btn-primary" onClick={add} disabled={!info || !!existing || adding || !id.trim() || !!preview?.filter_error}>
            {adding ? 'Adding…' : 'Track app'}
          </button>
        </div>
      </header>

      <form className="lookup-row" onSubmit={lookUp}>
        <input
          autoFocus
          placeholder="https://github.com/owner/repo, a GitLab project or a Codeberg repository"
          value={url}
          onChange={(e) => {
            setUrl(e.target.value);
            setUnknownHost(null);
          }}
          spellCheck={false}
        />
        {unknownHost && (
          <select value={forge} onChange={(e) => setForge(e.target.value as 'gitlab' | 'forgejo')} title={`What ${unknownHost} runs`}>
            <option value="forgejo">Forgejo / Gitea</option>
            <option value="gitlab">GitLab</option>
          </select>
        )}
        <button className="btn-secondary" type="submit" disabled={!url.trim()}>Look up</button>
      </form>
      {unknownHost && !info && (
        <p className="form-hint">Forager does not know what {unknownHost} runs. Choose its forge, then look up again.</p>
      )}
      {error && <p className="form-error">{error}</p>}
      {existing && (
        <p className="form-hint form-hint-warn">
          This project is already tracked as{' '}
          <button className="link-btn" onClick={() => setPage({ kind: 'app', id: existing.id })}>
            {existing.id}
          </button>
          .
        </p>
      )}

      {info && (
        <div className="page-columns">
          <div className="page-column">
            <SettingsSection
              draft={draft}
              onChange={setDraft}
              preview={preview}
              idField={
                <div className="form-group">
                  <label htmlFor="new-id">Id</label>
                  <input id="new-id" className="mono" value={id} onChange={(e) => setId(e.target.value)} spellCheck={false} />
                  <p className="form-hint">Names the app here and its files on disk: letters, digits, dots, dashes and underscores.</p>
                </div>
              }
            />
            <SignatureSection draft={signature} onChange={setSignature} app={null} forge={info.forge} />
          </div>
          <div className="page-column">
            <AssetsSection preview={preview} pinned={!!draft.format} loading={loading} error={previewError} onRefresh={refresh} />
          </div>
        </div>
      )}
    </div>
  );
}
