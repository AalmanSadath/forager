import { useEffect, useMemo } from 'react';
import { listen } from '@tauri-apps/api/event';
import * as ipc from './ipc';
import { useAppStore } from './store/appStore';
import { resolveAccent, resolveAppTheme } from './store/theme';
import { accentTokens } from './styles/accent';
import type { SystemAppearance } from './types';

/**
 * The app's palette and accent, followed live and applied to the page.
 * Trimmed from BifroSSH's hook of the same name.
 *
 * The accent is written onto the same elements that carry the theme class,
 * not onto <html>: the light and amoled palettes redeclare --accent on body
 * and .app, which would shadow an override on the root.
 */
export function useAppChrome(): { className: string; style: React.CSSProperties | undefined } {
  const settings = useAppStore((s) => s.settings);
  const systemAppearance = useAppStore((s) => s.systemAppearance);
  const setSystemAppearance = useAppStore((s) => s.setSystemAppearance);

  const resolvedTheme = resolveAppTheme(settings.app_theme, systemAppearance);
  const accent = resolveAccent(settings, systemAppearance);

  // Asked once, then followed: the backend relays the portal's changes.
  useEffect(() => {
    ipc.systemAppearance().then(setSystemAppearance).catch(() => {});
    const unlisten = listen<SystemAppearance>('system-appearance-changed', (e) => setSystemAppearance(e.payload));
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [setSystemAppearance]);

  useEffect(() => {
    const body = document.body;
    body.classList.remove('app-light', 'app-amoled');
    if (resolvedTheme === 'light') body.classList.add('app-light');
    else if (resolvedTheme === 'amoled') body.classList.add('app-amoled');
  }, [resolvedTheme]);

  const accentVars = useMemo(() => {
    const tokens = accent ? accentTokens(accent) : null;
    return tokens
      ? ({
          '--accent': tokens.accent,
          '--accent-hover': tokens.accentHover,
          '--on-accent': tokens.onAccent,
        } as React.CSSProperties)
      : undefined;
  }, [accent]);

  // Portals render into body rather than .app, so body carries the accent too.
  useEffect(() => {
    const body = document.body;
    const names = ['--accent', '--accent-hover', '--on-accent'];
    if (accentVars) {
      for (const [k, v] of Object.entries(accentVars)) body.style.setProperty(k, String(v));
    } else {
      for (const k of names) body.style.removeProperty(k);
    }
  }, [accentVars]);

  const className = `app${resolvedTheme === 'light' ? ' app-light' : resolvedTheme === 'amoled' ? ' app-amoled' : ''}`;
  return { className, style: accentVars };
}
