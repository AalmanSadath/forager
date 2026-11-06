import type { GuiSettings, ResolvedTheme, SystemAppearance, WindowFrame } from '../types';

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

const WINDOW_FRAME_CACHE = 'forager_window_frame';

/**
 * The window frame setting, mirrored in localStorage as BifroSSH does it:
 * the settings arrive from the backend after the first paint, and the window
 * should keep one frame from launch rather than switch when they load.
 */
export function cachedWindowFrame(): WindowFrame {
  try {
    return localStorage.getItem(WINDOW_FRAME_CACHE) === 'native' ? 'native' : 'custom';
  } catch {
    return 'custom';
  }
}

export function cacheWindowFrame(frame: WindowFrame) {
  try {
    localStorage.setItem(WINDOW_FRAME_CACHE, frame);
  } catch {
    // Only a cache: storage can be unavailable or full.
  }
}
