import { useState } from 'react';
import * as ipc from '../../ipc';
import { useAppStore } from '../../store/appStore';
import type { AppView, SourceInfo } from '../../types';
import Dropdown from '../shared/Dropdown';
import { FORGE_NAMES } from '../shared/icons';

type Kind = 'auto' | 'gitlab' | 'forgejo' | 'html' | 'jenkins' | 'direct';

/**
 * A local build's way to a source: look up its project's address, as on
 * the add page, and from then on it is checked and updated like any app.
 */
export default function LocalSourceSection({ app }: { app: AppView }) {
  const loadApps = useAppStore((s) => s.loadApps);
  const checkApps = useAppStore((s) => s.checkApps);
  const toast = useAppStore((s) => s.toast);
  const [url, setUrl] = useState('');
  const [kind, setKind] = useState<Kind>('auto');
  const [pattern, setPattern] = useState('');
  const [info, setInfo] = useState<SourceInfo | null>(null);
  const [unknownHost, setUnknownHost] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const lookUp = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setInfo(null);
    if (kind === 'html' && !pattern.trim()) {
      setError('A web page needs a pattern matching its download links.');
      return;
    }
    try {
      const parsed = await ipc.parseSource(url.trim(), kind === 'auto' ? null : kind, kind === 'html' ? pattern : null);
      setUnknownHost(parsed.unknown_host);
      setInfo(parsed.info);
    } catch (err) {
      setError(String(err));
    }
  };

  const use = async () => {
    if (!info) return;
    setBusy(true);
    setError(null);
    try {
      await ipc.setSource(app.id, info.spec);
      await loadApps();
      checkApps([app.id]);
      toast('success', `${app.name || app.id} now comes from ${info.display}.`);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="page-card">
      <h3>Project address</h3>
      <p className="form-hint">
        A local build is not checked for updates. Give it its project&rsquo;s address and Forager checks and updates it
        from there; the installed file stays until the first update replaces it.
      </p>
      <form className="lookup-row" onSubmit={lookUp}>
        <input
          placeholder="A GitHub, GitLab, Codeberg, SourceForge or SourceHut project, or a link to the file"
          aria-label="Project address"
          value={url}
          onChange={(e) => {
            setUrl(e.target.value);
            setInfo(null);
          }}
          spellCheck={false}
        />
        <Dropdown<Kind>
          value={kind}
          onChange={setKind}
          title="What the address is"
          ariaLabel="Source type"
          options={[
            { value: 'auto', label: 'Detect from address' },
            { value: 'gitlab', label: 'GitLab' },
            { value: 'forgejo', label: 'Forgejo / Gitea' },
            { value: 'html', label: 'Web page with links' },
            { value: 'jenkins', label: 'Jenkins job' },
            { value: 'direct', label: 'Direct link to the file' },
          ]}
        />
        <button className="btn-secondary" type="submit" disabled={!url.trim()}>
          Look up
        </button>
      </form>
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
        </div>
      )}
      {unknownHost && !info && (
        <p className="form-hint">
          Forager does not know what {unknownHost} runs. Choose GitLab, Forgejo / Gitea, Jenkins, a web page or a direct
          link, then look up again.
        </p>
      )}
      {info && (
        <div className="local-source-found">
          <span>
            {FORGE_NAMES[info.forge]} · <span className="mono">{info.display}</span>
          </span>
          <button className="btn-primary btn-sm" onClick={use} disabled={busy}>
            {busy ? 'Saving…' : 'Use this address'}
          </button>
        </div>
      )}
      {error && <p className="form-error">{error}</p>}
    </section>
  );
}
