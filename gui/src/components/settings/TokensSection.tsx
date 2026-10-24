import { useEffect, useState } from 'react';
import * as ipc from '../../ipc';
import { useAppStore } from '../../store/appStore';

const FIELDS: { key: string; label: string; hint: string }[] = [
  { key: 'GITHUB_TOKEN', label: 'GitHub', hint: 'Raises the limit from 60 to 5000 requests an hour. Needs no scopes for public projects.' },
  { key: 'GITLAB_TOKEN', label: 'GitLab.com', hint: 'Only sent to gitlab.com.' },
  { key: 'CODEBERG_TOKEN', label: 'Codeberg', hint: 'Only sent to codeberg.org.' },
];

/**
 * Forge API tokens, saved to ~/.config/forager/env where the CLI, this app
 * and the background check all read them. Other hosts take
 * FORAGER_TOKEN_<HOST> entries, kept as they are.
 */
export default function TokensSection() {
  const toast = useAppStore((s) => s.toast);
  const [values, setValues] = useState<Record<string, string> | null>(null);
  const [saved, setSaved] = useState<Record<string, string>>({});
  const [shown, setShown] = useState<string | null>(null);
  const [path, setPath] = useState('');

  useEffect(() => {
    ipc.getTokens().then((t) => {
      setValues(t);
      setSaved(t);
    });
    ipc.aboutInfo().then((a) => setPath(a.env_file));
  }, []);

  if (!values) return null;
  const dirty = JSON.stringify(values) !== JSON.stringify(saved);
  const extra = Object.keys(values).filter((k) => k.startsWith('FORAGER_TOKEN_'));

  const save = async () => {
    try {
      await ipc.setTokens(values);
      setSaved(values);
      toast('success', 'Tokens saved.');
    } catch (e) {
      toast('error', String(e));
    }
  };

  return (
    <section className="panel-section">
      <h3>Tokens</h3>
      {[...FIELDS, ...extra.map((key) => ({ key, label: key.replace('FORAGER_TOKEN_', '').toLowerCase().replace(/_/g, '.'), hint: 'Self-hosted forge.' }))].map(
        ({ key, label, hint }) => (
          <div className="form-group" key={key}>
            <label htmlFor={key}>{label}</label>
            <div className="input-with-eye">
              <input
                id={key}
                className="mono"
                type={shown === key ? 'text' : 'password'}
                value={values[key] ?? ''}
                placeholder="Not set"
                autoComplete="off"
                spellCheck={false}
                onChange={(e) => setValues({ ...values, [key]: e.target.value })}
              />
              <button
                type="button"
                className="eye-btn"
                onClick={() => setShown(shown === key ? null : key)}
                aria-label={shown === key ? 'Hide token' : 'Show token'}
              >
                {shown === key ? 'Hide' : 'Show'}
              </button>
            </div>
            <p className="form-hint">{hint}</p>
          </div>
        ),
      )}
      <p className="form-hint">
        Stored in <span className="mono">{path}</span>, readable only by you. Variables set in the environment win.
      </p>
      <div className="modal-actions" style={{ justifyContent: 'flex-start' }}>
        <button className="btn-primary" disabled={!dirty} onClick={save}>Save tokens</button>
      </div>
    </section>
  );
}
