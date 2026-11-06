import { getCurrentWindow } from '@tauri-apps/api/window';
import { useCustomFrame, useMaximized } from '../useWindowFrame';

/**
 * Minimize, maximize and close, drawn by the app when the window has no
 * system frame, as in BifroSSH. Renders nothing when it has one.
 *
 * Close goes through the window's close request, as the system button would,
 * so whatever the window does on closing still runs.
 */
export default function WindowControls() {
  const custom = useCustomFrame();
  const maximized = useMaximized(custom);

  if (!custom) return null;
  const win = getCurrentWindow();

  return (
    <div className="window-controls">
      <button className="window-control" aria-label="Minimize" title="Minimize" onClick={() => void win.minimize()}>
        <svg width="10" height="10" viewBox="0 0 10 10">
          <line x1="0" y1="5" x2="10" y2="5" stroke="currentColor" strokeWidth="1.2" />
        </svg>
      </button>
      <button
        className="window-control"
        aria-label={maximized ? 'Restore' : 'Maximize'}
        title={maximized ? 'Restore' : 'Maximize'}
        onClick={() => void win.toggleMaximize()}
      >
        {maximized ? (
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.2">
            <rect x="0.6" y="2.6" width="6.8" height="6.8" />
            <path d="M2.6 2.6V0.6h6.8v6.8H7.4" />
          </svg>
        ) : (
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.2">
            <rect x="0.6" y="0.6" width="8.8" height="8.8" />
          </svg>
        )}
      </button>
      <button
        className="window-control window-control-close"
        aria-label="Close"
        title="Close"
        onClick={() => void win.close()}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" stroke="currentColor" strokeWidth="1.2">
          <line x1="0" y1="0" x2="10" y2="10" />
          <line x1="10" y1="0" x2="0" y2="10" />
        </svg>
      </button>
    </div>
  );
}
