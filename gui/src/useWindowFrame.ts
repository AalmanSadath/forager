import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useAppStore } from './store/appStore';

/**
 * The window frame, as BifroSSH draws it. Trimmed of its Android case and of
 * the resize band, which Tauri 2.12 no longer needs.
 */

/** Whether this window draws its own minimize, maximize and close buttons. */
export function useCustomFrame(): boolean {
  return useAppStore((s) => s.settings.window_frame) === 'custom';
}

/**
 * Keeps the window's system frame in step with the setting, live.
 *
 * The window is configured without a frame. Before the settings load, the
 * setting holds the value cached from the last run, so a user who chose the
 * native frame gets it back as soon as the page starts.
 */
export function useWindowFrame(): void {
  const custom = useCustomFrame();
  useEffect(() => {
    getCurrentWindow()
      .setDecorations(!custom)
      .catch(() => {});
  }, [custom]);
}

/** Whether this window is maximized, followed while `watch` is on. */
export function useMaximized(watch: boolean): boolean {
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    if (!watch) return;
    const win = getCurrentWindow();
    const check = () => {
      win
        .isMaximized()
        .then(setMaximized)
        .catch(() => {});
    };
    check();
    const unlisten = win.onResized(check);
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [watch]);
  return maximized;
}
