/** Shapes the Rust side sends, mirroring `src-tauri/src/commands/*.rs`. */

export type Format = 'deb' | 'rpm' | 'appimage' | 'tarball' | 'flatpak' | 'pacman';
export type Forge = 'github' | 'gitlab' | 'forgejo' | 'html';

export interface InstalledView {
  tag: string;
  format: Format;
  asset: string;
  package: string | null;
  path: string | null;
  links: string[];
}

/** A source as the backend stores it; opaque here, passed back as is. */
export type SourceSpec = { kind: Forge } & Record<string, string>;

export interface AppView {
  id: string;
  spec: SourceSpec;
  source: string;
  web_url: string;
  forge: Forge;
  format: Format | null;
  asset_filter: string | null;
  prerelease: boolean;
  bins: string[];
  signature: 'minisign' | 'gpg' | 'attestation' | null;
  minisign_key: string | null;
  gpg_keyring: string | null;
  installed: InstalledView | null;
}

export interface SystemAppearance {
  color_scheme: 'dark' | 'light' | 'no-preference';
  accent: string | null;
}

export type AppTheme = 'system' | 'dark' | 'light' | 'amoled';
export type ResolvedTheme = Exclude<AppTheme, 'system'>;

export interface GuiSettings {
  app_theme: AppTheme;
  /** `#rrggbb`, or null to follow the desktop. */
  accent_color: string | null;
}

export type UpdateState = 'not_installed' | 'missing' | 'wrong_format' | 'outdated' | 'up_to_date';

export interface StatusView {
  state: UpdateState;
  latest_tag: string;
  latest_name: string | null;
  published_at: string | null;
  asset: string | null;
  asset_format: Format | null;
  asset_error: string | null;
  repo_version: string | null;
  checked_at: number;
}

/** Payload of the `app-status` event. */
export interface StatusEvent {
  id: string;
  status: StatusView | null;
  error: string | null;
}

export type StartRequest = { kind: 'add'; url: string } | { kind: 'open'; id: string };

export interface SourceInfo {
  spec: SourceSpec;
  display: string;
  web_url: string;
  forge: Forge;
  default_id: string;
}

export interface ParsedSource {
  info: SourceInfo | null;
  /** Set when the host's forge kind has to be chosen. */
  unknown_host: string | null;
}

export interface SettingsInput {
  format: Format | null;
  asset_filter: string | null;
  prerelease: boolean;
  bins: string[];
  /** For web-page sources: the download link pattern. */
  pattern: string | null;
}

export type Reason =
  | 'not_package'
  | 'filter'
  | 'debug'
  | 'other_os'
  | 'format_not_wanted'
  | 'other_arch'
  | 'source_archive';

export type Verdict = { kind: 'chosen' } | { kind: 'usable' } | { kind: 'rejected'; reason: Reason };

export interface CandidateView {
  name: string;
  size: number;
  format: Format | null;
  verdict: Verdict;
}

export interface ReleaseView {
  tag: string;
  name: string | null;
  published_at: string | null;
  prerelease: boolean;
}

export interface Preview {
  release: ReleaseView;
  candidates: CandidateView[];
  filter_error: string | null;
}

export type SignatureForm =
  | { kind: 'keep' }
  | { kind: 'none' }
  | { kind: 'minisign'; key: string }
  | { kind: 'gpg'; key_file: string }
  | { kind: 'attestation' };

export interface SummaryLine {
  label: string;
  value: string;
  tone: 'normal' | 'warn' | 'danger';
}

export interface PlanView {
  from: string | null;
  to: string;
  missing: boolean;
  format: Format;
  lines: SummaryLine[];
  unverified: boolean;
  needs_root: boolean;
}

export interface DownloadedView {
  package: string | null;
  repo_version: string | null;
  first_install: boolean;
}

/** Payload of `update-progress`. */
export interface ProgressEvent {
  id: string;
  phase: 'download' | 'install';
  done: number;
  total: number | null;
  percent: number | null;
}

export type EventKind = 'installed' | 'updated' | 'removed' | 'failed' | 'updates_found';

export interface HistoryEvent {
  time: number;
  app: string;
  kind: EventKind;
  via: 'cli' | 'gui' | 'timer';
  from?: string;
  to?: string;
  format?: Format;
  message?: string;
}

export interface TimerState {
  enabled: boolean;
  auto_update: boolean;
  next_run: string | null;
  cli: string | null;
}

export interface AboutInfo {
  version: string;
  config: string;
  env_file: string;
  history: string;
  cache: string;
  keys: string;
  apps_dir: string | null;
  opt_dir: string | null;
}
