import { useCallback, useEffect, useRef, useState } from 'react';
import * as ipc from '../../ipc';
import type { AppView, Format, Preview, SettingsInput, SignatureForm, SourceSpec } from '../../types';

/** The settings form as typed, before it becomes a `SettingsInput`. */
export interface SettingsDraft {
  format: Format | '';
  filter: string;
  /** Regex a release tag must match. */
  tagFilter: string;
  prerelease: boolean;
  /** Comma-separated paths. */
  bins: string;
  /** Download link pattern; only web-page apps have one. */
  pattern: string;
}

export const EMPTY_DRAFT: SettingsDraft = { format: '', filter: '', tagFilter: '', prerelease: false, bins: '', pattern: '' };

export function draftOf(app: AppView): SettingsDraft {
  return {
    format: app.format ?? '',
    filter: app.asset_filter ?? '',
    tagFilter: app.tag_filter ?? '',
    prerelease: app.prerelease,
    bins: app.bins.join(', '),
    pattern: app.spec.kind === 'html' ? (app.spec.pattern ?? '') : '',
  };
}

export function toInput(d: SettingsDraft): SettingsInput {
  return {
    format: d.format || null,
    asset_filter: d.filter.trim() || null,
    tag_filter: d.tagFilter.trim() || null,
    prerelease: d.prerelease,
    bins: d.bins
      .split(',')
      .map((b) => b.trim())
      .filter(Boolean),
    pattern: d.pattern.trim() || null,
  };
}

export function sameDraft(a: SettingsDraft, b: SettingsDraft): boolean {
  return JSON.stringify(toInput(a)) === JSON.stringify(toInput(b));
}

/** The signature choice as the form holds it. */
export interface SignatureDraft {
  kind: 'none' | 'minisign' | 'gpg' | 'attestation';
  minisignKey: string;
  /** A newly picked key file; empty keeps the stored one. */
  gpgFile: string;
}

export function signatureDraftOf(app: AppView | null): SignatureDraft {
  return { kind: app?.signature ?? 'none', minisignKey: app?.minisign_key ?? '', gpgFile: '' };
}

/** What to send: `keep` when nothing changed, so stored keys stay put. */
export function signatureForm(d: SignatureDraft, app: AppView | null): SignatureForm {
  const current = signatureDraftOf(app);
  const unchanged =
    d.kind === current.kind &&
    (d.kind !== 'minisign' || d.minisignKey.trim() === current.minisignKey) &&
    (d.kind !== 'gpg' || d.gpgFile === '');
  if (unchanged) return { kind: 'keep' };
  switch (d.kind) {
    case 'none':
      return { kind: 'none' };
    case 'minisign':
      return { kind: 'minisign', key: d.minisignKey.trim() };
    case 'gpg':
      return { kind: 'gpg', key_file: d.gpgFile };
    case 'attestation':
      return { kind: 'attestation' };
  }
}

/**
 * The latest release as `settings` would see it, refreshed as the user types
 * (debounced; the release itself is cached on the Rust side).
 */
export function usePreview(spec: SourceSpec | null, settings: SettingsInput, id: string | null = null) {
  const [preview, setPreview] = useState<Preview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const key = JSON.stringify(settings);
  const latest = useRef(0);

  const run = useCallback(
    (refresh: boolean) => {
      if (!spec) return;
      const ticket = ++latest.current;
      setLoading(true);
      ipc
        .preview(spec, JSON.parse(key) as SettingsInput, refresh, id)
        .then((p) => {
          if (ticket !== latest.current) return;
          setPreview(p);
          setError(null);
        })
        .catch((e) => {
          if (ticket !== latest.current) return;
          setError(String(e));
        })
        .finally(() => {
          if (ticket === latest.current) setLoading(false);
        });
    },
    [spec, key, id],
  );

  useEffect(() => {
    const timer = setTimeout(() => run(false), 250);
    return () => clearTimeout(timer);
  }, [run]);

  return { preview, error, loading, refresh: () => run(true) };
}
