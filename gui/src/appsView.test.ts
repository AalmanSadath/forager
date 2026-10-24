import { describe, expect, it } from 'vitest';
import { cardModel, countUpdates, formatsInUse, groupApps, restoreChecks, type CheckEntry } from './appsView';
import type { AppView, StatusView } from './types';

function app(id: string, extra: Partial<AppView> = {}): AppView {
  return {
    id,
    spec: { kind: 'github', owner: 'o', repo: id },
    source: `github.com/o/${id}`,
    web_url: `https://github.com/o/${id}`,
    forge: 'github',
    format: null,
    asset_filter: null,
    tag_filter: null,
    prerelease: false,
    bins: [],
    signature: null,
    minisign_key: null,
    gpg_keyring: null,
    installed: null,
    ...extra,
    installs: extra.installs ?? (extra.installed ? [extra.installed] : []),
  };
}

const installed = (tag: string, format: AppView['format'] = 'tarball') =>
  ({ tag, format: format!, asset: 'x', package: null, path: null, links: [] });

function status(state: StatusView['state'], latest = '2.0'): CheckEntry {
  return {
    checking: false,
    error: null,
    status: {
      state,
      latest_tag: latest,
      latest_name: null,
      published_at: null,
      asset: 'tool.AppImage',
      asset_format: 'appimage',
      asset_error: null,
      repo_version: null,
      checked_at: 0,
      installs: [],
      found: false,
    },
  };
}

describe('cardModel', () => {
  it('shows an update as old → new with the update action', () => {
    const m = cardModel(app('a', { installed: installed('1.0') }), status('outdated'));
    expect(m).toMatchObject({ section: 'updates', dot: 'update', versions: '1.0 → 2.0', action: 'update', format: 'tarball' });
  });

  it('puts fetch errors and missing installs under problems', () => {
    expect(cardModel(app('a'), { checking: false, status: null, error: 'project not found' })).toMatchObject({
      section: 'problems',
      dot: 'error',
      versions: 'project not found',
    });
    expect(cardModel(app('a', { installed: installed('1.0') }), status('missing'))).toMatchObject({
      section: 'problems',
      action: 'reinstall',
    });
  });

  it('uses the format an install would get when nothing is installed', () => {
    expect(cardModel(app('a'), status('not_installed'))).toMatchObject({
      section: 'not_installed',
      format: 'appimage',
      versions: '2.0 available',
    });
  });

  it('pulses while checking but keeps its section', () => {
    expect(cardModel(app('a', { installed: installed('2.0') }), { ...status('up_to_date'), checking: true })).toMatchObject({
      section: 'installed',
      dot: 'busy',
    });
  });

  it('places an unchecked app by whether it is installed', () => {
    expect(cardModel(app('a', { installed: installed('1.0') }), undefined).section).toBe('installed');
    expect(cardModel(app('a'), undefined).section).toBe('not_installed');
  });
});

describe('apps installed in several formats', () => {
  const rpm = { ...installed('2.0', 'rpm'), package: 'tool' };
  const flatpak = { ...installed('1.0', 'flatpak'), package: 'org.example.Tool' };
  const both = app('tool', { installed: rpm, installs: [rpm, flatpak] });
  const entry = (): CheckEntry => {
    const e = status('outdated');
    e.status!.installs = [
      { format: 'rpm', tag: '2.0', state: 'up_to_date', repo_version: null },
      { format: 'flatpak', tag: '1.0', state: 'outdated', repo_version: null },
    ];
    return e;
  };

  it('lists every format and shows the install that needs updating', () => {
    const m = cardModel(both, entry());
    expect(m.formats).toEqual(['rpm', 'flatpak']);
    expect(m).toMatchObject({ section: 'updates', versions: '1.0 → 2.0', format: 'flatpak' });
  });

  it('is found by any of its formats and package names', () => {
    const entries = { tool: entry() };
    expect(groupApps([both], entries, 'flatpak', '')).toHaveLength(1);
    expect(groupApps([both], entries, 'rpm', '')).toHaveLength(1);
    expect(groupApps([both], entries, 'all', 'org.example')).toHaveLength(1);
    expect(formatsInUse([both], entries)).toEqual(['rpm', 'flatpak']);
  });
});

describe('groupApps', () => {
  const apps = [
    app('ripgrep', { installed: installed('1.0') }),
    app('gearlever', { installed: installed('4.6', 'flatpak') }),
    app('librewolf'),
  ];
  const entries = {
    ripgrep: status('outdated'),
    gearlever: status('up_to_date'),
    librewolf: status('not_installed'),
  };

  it('orders sections updates, problems, installed, not installed', () => {
    expect(groupApps(apps, entries, 'all', '').map((s) => s.key)).toEqual(['updates', 'installed', 'not_installed']);
  });

  it('filters by section chip, format chip and every search word', () => {
    expect(groupApps(apps, entries, 'updates', '').flatMap((s) => s.items.map((i) => i.app.id))).toEqual(['ripgrep']);
    expect(groupApps(apps, entries, 'flatpak', '').flatMap((s) => s.items.map((i) => i.app.id))).toEqual(['gearlever']);
    expect(groupApps(apps, entries, 'all', 'GITHUB rip').flatMap((s) => s.items.map((i) => i.app.id))).toEqual(['ripgrep']);
    expect(groupApps(apps, entries, 'all', 'nothing-matches')).toEqual([]);
  });

  it('counts updates and lists formats in a fixed order', () => {
    expect(countUpdates(apps, entries)).toBe(1);
    expect(formatsInUse(apps, entries)).toEqual(['flatpak', 'appimage', 'tarball']);
  });
});

describe('restoreChecks', () => {
  it('keeps checks saved by this version and drops older or broken ones', () => {
    const current = status('up_to_date');
    const old = status('up_to_date');
    delete (old.status as Partial<StatusView>).installs;
    const restored = restoreChecks(JSON.stringify({ a: { ...current, checking: true }, b: old, c: { status: null } }));
    expect(Object.keys(restored)).toEqual(['a']);
    expect(restored.a.checking).toBe(false);
    expect(restoreChecks('not json')).toEqual({});
    expect(restoreChecks(null)).toEqual({});
  });
});
