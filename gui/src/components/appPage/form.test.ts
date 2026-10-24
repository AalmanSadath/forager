import { describe, expect, it } from 'vitest';
import { draftOf, sameDraft, signatureForm, toInput } from './form';
import type { AppView } from '../../types';

const app: AppView = {
  id: 'tool',
  spec: { kind: 'github', owner: 'o', repo: 'tool' },
  source: 'github.com/o/tool',
  web_url: 'https://github.com/o/tool',
  forge: 'github',
  format: 'tarball',
  asset_filter: 'musl',
  prerelease: false,
  bins: ['bin/tool', 'bin/helper'],
  signature: 'minisign',
  minisign_key: 'RWQkey',
  gpg_keyring: null,
  installed: null,
};

describe('settings form', () => {
  it('round-trips an app and trims what was typed', () => {
    expect(toInput(draftOf(app))).toEqual({
      format: 'tarball',
      asset_filter: 'musl',
      prerelease: false,
      bins: ['bin/tool', 'bin/helper'],
    });
    expect(toInput({ format: '', filter: '  ', prerelease: true, bins: ' a ,, b ' })).toEqual({
      format: null,
      asset_filter: null,
      prerelease: true,
      bins: ['a', 'b'],
    });
  });

  it('ignores edits that change nothing that is saved', () => {
    expect(sameDraft(draftOf(app), { ...draftOf(app), bins: 'bin/tool,bin/helper ' })).toBe(true);
    expect(sameDraft(draftOf(app), { ...draftOf(app), filter: 'gnu' })).toBe(false);
  });
});

describe('signature form', () => {
  const current = { kind: 'minisign' as const, minisignKey: 'RWQkey', gpgFile: '' };

  it('keeps the stored signature when nothing changed', () => {
    expect(signatureForm(current, app)).toEqual({ kind: 'keep' });
    expect(signatureForm({ ...current, minisignKey: ' RWQkey ' }, app)).toEqual({ kind: 'keep' });
  });

  it('sends a new key, a new choice, or none', () => {
    expect(signatureForm({ ...current, minisignKey: 'RWQother' }, app)).toEqual({ kind: 'minisign', key: 'RWQother' });
    expect(signatureForm({ ...current, kind: 'none' }, app)).toEqual({ kind: 'none' });
    expect(signatureForm({ kind: 'gpg', minisignKey: '', gpgFile: '/k.asc' }, null)).toEqual({ kind: 'gpg', key_file: '/k.asc' });
  });
});
