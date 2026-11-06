import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useAppStore } from './store/appStore';

/** The window frame, as BifroSSH draws it, trimmed of its Android case. */

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

/**
 * How wide, in CSS pixels, the band along the window's edge is where Tauri
 * resizes a frameless window, or 0 when there is none: a framed window or a
 * maximized one.
 *
 * Since Tauri 2.12 the band takes the pointer before the page sees it, at 5
 * pixels times the desktop's whole scale factor. WebKitGTK's overlay
 * scrollbars only answer to the pointer within a few pixels of their own
 * edge, so one along the window's right edge could not be reached at all;
 * the pages keep their scroll areas this far in. `devicePixelRatio` also
 * carries fractional scaling and zoom, so this only approximates GDK's
 * integer scale.
 */
export function useResizeBand(): number {
  const custom = useCustomFrame();
  const maximized = useMaximized(custom);
  if (!custom || maximized) return 0;
  return 5 * Math.max(1, Math.floor(window.devicePixelRatio));
}
