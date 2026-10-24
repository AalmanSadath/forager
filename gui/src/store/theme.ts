import type { GuiSettings, ResolvedTheme, SystemAppearance } from '../types';

/**
 * Theme resolution, as BifroSSH's themeCache.ts does it.
 *
 * Only an explicit "dark" from the desktop means dark: GNOME reports its light
 * appearance as "no preference", never as light.
 */
export function resolveAppTheme(setting: GuiSettings['app_theme'], system: SystemAppearance): ResolvedTheme {
  if (setting !== 'system') return setting;
  return system.color_scheme === 'dark' ? 'dark' : 'light';
}

/** The user's accent, else the desktop's, else none (the palette's own). */
export function resolveAccent(settings: Pick<GuiSettings, 'accent_color'>, system: SystemAppearance): string | null {
  return settings.accent_color ?? system.accent;
}

export const NO_APPEARANCE: SystemAppearance = { color_scheme: 'no-preference', accent: null };

/** Each palette's own accent, as tokens.css declares it. */
export const THEME_ACCENTS: Record<ResolvedTheme, string> = {
  dark: '#58a6ff',
  light: '#155397',
  amoled: '#a78bfa',
};
