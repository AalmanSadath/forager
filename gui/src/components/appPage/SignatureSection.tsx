import { open } from '@tauri-apps/plugin-dialog';
import type { AppView, Forge } from '../../types';
import type { SignatureDraft } from './form';

const KINDS: { kind: SignatureDraft['kind']; label: string }[] = [
  { kind: 'none', label: 'None' },
  { kind: 'minisign', label: 'Minisign' },
  { kind: 'gpg', label: 'GPG' },
  { kind: 'attestation', label: 'Attestation' },
];

/** The key releases must be signed with. */
export default function SignatureSection({
  draft,
  onChange,
  app,
  forge,
}: {
  draft: SignatureDraft;
  onChange: (d: SignatureDraft) => void;
  app: AppView | null;
  forge: Forge;
}) {
  const set = (patch: Partial<SignatureDraft>) => onChange({ ...draft, ...patch });
  const pickKey = async () => {
    const file = await open({ multiple: false, directory: false, title: 'OpenPGP public key' });
    if (typeof file === 'string') set({ gpgFile: file });
  };

  return (
    <section className="page-card">
      <h3>Signature</h3>
      <div className="toggle-row" style={{ maxWidth: 420 }}>
        {KINDS.map(({ kind, label }) => (
          <button
            key={kind}
            type="button"
            className={`toggle-btn${draft.kind === kind ? ' active' : ''}`}
            onClick={() => set({ kind })}
            disabled={kind === 'attestation' && forge !== 'github'}
            title={kind === 'attestation' && forge !== 'github' ? 'Only for GitHub projects' : undefined}
          >
            {label}
          </button>
        ))}
      </div>
      {draft.kind === 'none' && (
        <p className="form-hint">
          A checksum published next to an asset only shows the download is intact. Give the project's
          signing key to refuse anything it did not sign.
        </p>
      )}
      {draft.kind === 'minisign' && (
        <div className="form-group signature-field">
          <label htmlFor="minisign">Public key</label>
          <textarea
            id="minisign"
            rows={2}
            placeholder="RW… or the contents of minisign.pub"
            value={draft.minisignKey}
            onChange={(e) => set({ minisignKey: e.target.value })}
            spellCheck={false}
          />
          <p className="form-hint">Assets need a .minisig next to them, or a signed checksum list.</p>
        </div>
      )}
      {draft.kind === 'gpg' && (
        <div className="form-group signature-field">
          <label>Public key</label>
          <div className="key-file-row">
            <button className="btn-secondary btn-sm" type="button" onClick={pickKey}>Choose key file…</button>
            <span className="mono key-file">
              {draft.gpgFile || (app?.gpg_keyring ? `Stored: ${app.gpg_keyring}` : 'No key chosen')}
            </span>
          </div>
          <p className="form-hint">
            Get it from the project's website, not from the release it checks. Kept in Forager's own
            keyring; nothing is added to yours.
          </p>
        </div>
      )}
      {draft.kind === 'attestation' && (
        <p className="form-hint">
          Checked with <span className="mono">gh attestation verify</span>, which needs the GitHub CLI and
          <span className="mono"> gh auth login</span>.
        </p>
      )}
    </section>
  );
}
