/** Shapes the Rust side sends, mirroring `src-tauri/src/commands/*.rs`. */

export type Format = 'deb' | 'rpm' | 'appimage' | 'tarball' | 'flatpak' | 'pacman';
export type Forge = 'github' | 'gitlab' | 'forgejo' | 'html' | 'sourceforge' | 'sourcehut' | 'jenkins' | 'direct';

export interface InstalledView {
  tag: string;
  format: Format;
  asset: string;
  package: string | null;
  path: string | null;
  links: string[];
  /** Whether an AppImage or tarball is in the menu; `unavailable` for other formats. */
  menu_entry: 'present' | 'missing' | 'unavailable';
}

export interface MenuEntryResult {
  id: string;
  error: string | null;
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
  tag_filter: string | null;
  /** Versions to follow, as a semver requirement such as `<25`. */
  versions: string | null;
  /** Days a release must be out before background updates install it. */
  cooldown_days: number;
  prerelease: boolean;
  bins: string[];
  /** A release the user chose not to install. */
  skipped: string | null;
  /** Only new releases are reported; Forager does not install the app. */
  track_only: boolean;
  /** For a tracked-only app, the release the user last saw. */
  seen: string | null;
  signature: 'minisign' | 'gpg' | 'attestation' | null;
  minisign_key: string | null;
  gpg_keyring: string | null;
  /** The install in the preferred format. */
  installed: InstalledView | null;
  /** Every install, the preferred one first. */
  installs: InstalledView[];
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
  /** Each install and how it compares with the latest release. */
  installs: InstallStatusView[];
  /** The latest release is the skipped one. */
  skipped: boolean;
  /** The latest release's page on the forge. */
  release_url: string;
  /** When background updates may install the latest release, while the cooldown holds it back. */
  waits_until: string | null;
  /** Installs were found on the system, removed, or at a new version, and saved. */
  found: boolean;
}

export interface InstallStatusView {
  format: Format;
  tag: string;
  state: UpdateState;
  repo_version: string | null;
}

/** Payload of the `app-status` event. */
export interface StatusEvent {
  id: string;
  status: StatusView | null;
  error: string | null;
}

/** What a `forager://add?…` link asks to add, and the settings to start from. */
export interface AddRequest {
  url: string;
  forge: string | null;
  pattern: string | null;
  id: string | null;
  asset_filter: string | null;
  tag_filter: string | null;
  versions: string | null;
  cooldown_days: number;
  format: Format | null;
  bins: string[];
  prerelease: boolean;
  track_only: boolean;
}

/** What an AppImage downloaded by hand says about itself. */
export interface AppImageInfo {
  name: string | null;
  version: string | null;
  /** Raw update information, e.g. `gh-releases-zsync|owner|repo|latest|…`. */
  update_info: string | null;
  /** The project it names, as the Add page takes it. */
  source: AddRequest | null;
}

/** An AppImage chosen on the Add page's way in. */
export interface AppImageFile {
  path: string;
  name: string | null;
  version: string | null;
  /** Its raw update information, if any. */
  info: string | null;
}

/** An AppImage the user has, to become the new app's install. */
export interface AdoptInput {
  path: string;
  version: string | null;
  /** Copy the file rather than move it. */
  keep: boolean;
}

export type StartRequest =
  | { kind: 'add'; url: string; link: AddRequest | null }
  | { kind: 'open'; id: string }
  | { kind: 'adopt'; path: string };

export interface ImportResult {
  id: string;
  error: string | null;
}

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
  tag_filter: string | null;
  versions: string | null;
  cooldown_days: number;
  prerelease: boolean;
  bins: string[];
  /** For web-page sources: the download link pattern. */
  pattern: string | null;
  /** Only report new releases instead of installing them. */
  track_only: boolean;
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
  /** The app's install updating would replace with this asset. */
  updates: Format | null;
}

export interface ReleaseView {
  tag: string;
  name: string | null;
  published_at: string | null;
  prerelease: boolean;
  /** Release notes, usually markdown. */
  notes: string | null;
}

export interface Preview {
  release: ReleaseView;
  candidates: CandidateView[];
  filter_error: string | null;
  tag_filter_error: string | null;
  versions_error: string | null;
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
  /** Older than the install it replaces. */
  downgrade: boolean;
  /** Release skipped once this is installed. */
  skip_after: string | null;
  notes: string | null;
}

export interface FinishedView {
  app: AppView;
  /** Forager updated itself; this window still runs the old version. */
  restart_needed: boolean;
}

/** One planned update of an app, or why it cannot be made. */
export interface PlanResult {
  plan: PlanView | null;
  error: string | null;
}

export interface DownloadedView {
  package: string | null;
  repo_version: string | null;
  first_install: boolean;
  /** How a delta download went, when there was one. */
  delta: string | null;
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
  skip_metered: boolean;
  skip_on_battery: boolean;
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
