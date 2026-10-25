import { useEffect, useMemo, useRef, useState } from 'react';
import * as ipc from '../ipc';
import { useAppStore } from '../store/appStore';
import type { AddRequest, AppImageFile, SourceInfo } from '../types';
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
export default function AddAppPage({
  initialUrl,
  link,
  adopt,
}: {
  initialUrl?: string;
  link?: AddRequest;
  /** An AppImage the user has, to become the app's install. */
  adopt?: AppImageFile;
}) {
  const setPage = useAppStore((s) => s.setPage);
  const loadApps = useAppStore((s) => s.loadApps);
  const checkApps = useAppStore((s) => s.checkApps);
  const toast = useAppStore((s) => s.toast);
  const apps = useAppStore((s) => s.apps);

  const [url, setUrl] = useState(initialUrl ?? '');
  const [unknownHost, setUnknownHost] = useState<string | null>(null);
  // What the address is: worked out from the host, or said by the user.
  const [kind, setKind] = useState<Kind>(() => kindOf(link?.forge));
  const [pattern, setPattern] = useState(link?.pattern ?? '');
  const [info, setInfo] = useState<SourceInfo | null>(null);
  const [id, setId] = useState('');
  const [draft, setDraft] = useState<SettingsDraft>(() => (link ? draftOfLink(link) : EMPTY_DRAFT));
  const [signature, setSignature] = useState<SignatureDraft>({ kind: 'none', minisignKey: '', gpgFile: '' });
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  // Move the AppImage into ~/Applications, or leave the original where it is.
  const [keep, setKeep] = useState(false);

  const input = useMemo(() => toInput(draft), [draft]);
  // The store refuses a second app for the same project, so say so up front.
  const existing = info ? apps.find((a) => a.source === info.display) : undefined;
  const { preview, error: previewError, loading, refresh } = usePreview(info?.spec ?? null, input);

  const lookUp = async (e?: React.FormEvent) => {
    e?.preventDefault();
    setError(null);
    try {
      if (kind === 'html' && !pattern.trim()) {
        setError('A web page needs a pattern matching its download links.');
        return;
      }
      const parsed = await ipc.parseSource(url.trim(), kind === 'auto' ? null : kind, kind === 'html' ? pattern : null);
      setUnknownHost(parsed.unknown_host);
      setInfo(parsed.info);
      if (parsed.info) setId(link?.id ?? parsed.info.default_id);
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
      const app = await ipc.addApp(
        info.spec,
        id.trim(),
        input,
        signatureForm(signature, null),
        adopt ? { path: adopt.path, version: adopt.version, keep } : null,
      );
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
          <h1>{adopt ? 'Add AppImage' : 'Add app'}</h1>
          {info && <span className="page-header-sub">{FORGE_NAMES[info.forge]} · {info.display}</span>}
        </div>
        <div className="page-header-actions">
          <button className="btn-primary" onClick={add} disabled={!info || !!existing || adding || !id.trim() || !!preview?.filter_error}>
            {adding ? 'Adding…' : adopt ? 'Track AppImage' : 'Track app'}
          </button>
        </div>
      </header>

      <form className="lookup-row" onSubmit={lookUp}>
        <input
          autoFocus
          placeholder="A GitHub, GitLab, Codeberg, SourceForge or SourceHut project, or a link to the file"
          value={url}
          onChange={(e) => {
            setUrl(e.target.value);
            setUnknownHost(null);
          }}
          spellCheck={false}
        />
        <select
          value={kind}
          onChange={(e) => setKind(e.target.value as typeof kind)}
          title="What the address is"
          aria-label="Source type"
        >
          <option value="auto">Detect from address</option>
          <option value="gitlab">GitLab</option>
          <option value="forgejo">Forgejo / Gitea</option>
          <option value="html">Web page with links</option>
          <option value="jenkins">Jenkins job</option>
          <option value="direct">Direct link to the file</option>
        </select>
        <button className="btn-secondary" type="submit" disabled={!url.trim()}>Look up</button>
      </form>
      {adopt && (
        <section className="page-card adopt-card">
          <h3>AppImage</h3>
          <dl className="facts">
            <dt>File</dt>
            <dd className="mono">{adopt.path}</dd>
            {adopt.name && (
              <>
                <dt>App</dt>
                <dd>{adopt.name}</dd>
              </>
            )}
            <dt>Version</dt>
            <dd className="mono">{adopt.version ?? 'Unknown'}</dd>
          </dl>
          <p className="form-hint">
            {link
              ? 'Its project comes from the update information inside it. Updates come from there from now on.'
              : "It does not say where it comes from: paste its project's address above and look it up."}
          </p>
          <label className="checkbox-row">
            <input type="checkbox" checked={keep} onChange={(e) => setKeep(e.target.checked)} />
            <span>Keep the original file (copy it to ~/Applications instead of moving it)</span>
          </label>
        </section>
      )}
      {link && !adopt && (
        <p className="form-hint">
          Opened from a forager:// link. Check the address and settings before tracking the app; a signing key, if it
          has one, comes from the project, never from a link.
        </p>
      )}
      {kind === 'html' && (
        <div className="form-group pattern-field">
          <input
            className="mono"
            placeholder="Pattern for download links, e.g. app-[0-9.]+-x86_64\.AppImage$"
            aria-label="Download link pattern"
            value={pattern}
            onChange={(e) => setPattern(e.target.value)}
            spellCheck={false}
          />
          <p className="form-hint">
            A regular expression over each link's full address. Matching links are the downloads; the version is the
            first number in the file name, or a group named version: (?P&lt;version&gt;…).
          </p>
        </div>
      )}
      {unknownHost && !info && (
        <p className="form-hint">
          Forager does not know what {unknownHost} runs. Choose GitLab, Forgejo / Gitea, Jenkins, a web page or a
          direct link, then look up again.
        </p>
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

type Kind = 'auto' | 'gitlab' | 'forgejo' | 'html' | 'jenkins' | 'direct';

/** The source menu's choice for a link's `forge`; others are found from the address. */
function kindOf(forge: string | null | undefined): Kind {
  return forge === 'gitlab' || forge === 'forgejo' || forge === 'html' || forge === 'jenkins' || forge === 'direct'
    ? forge
    : 'auto';
}

/** The settings form as a link fills it in. */
function draftOfLink(link: AddRequest): SettingsDraft {
  return {
    ...EMPTY_DRAFT,
    format: link.format ?? '',
    filter: link.asset_filter ?? '',
    tagFilter: link.tag_filter ?? '',
    prerelease: link.prerelease,
    bins: link.bins.join(', '),
    pattern: link.pattern ?? '',
    trackOnly: link.track_only,
  };
}
