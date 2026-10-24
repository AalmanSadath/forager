/** Shapes the Rust side sends, mirroring `src-tauri/src/commands/*.rs`. */

export type Format = 'deb' | 'rpm' | 'appimage' | 'tarball' | 'flatpak' | 'pacman';
export type Forge = 'github' | 'gitlab' | 'forgejo';

export interface InstalledView {
  tag: string;
  format: Format;
  asset: string;
  package: string | null;
  path: string | null;
  links: string[];
}

export interface AppView {
  id: string;
  source: string;
  web_url: string;
  forge: Forge;
  format: Format | null;
  asset_filter: string | null;
  prerelease: boolean;
  bins: string[];
  signature: 'minisign' | 'gpg' | 'attestation' | null;
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
