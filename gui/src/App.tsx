import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { countUpdates } from './appsView';
import type { StatusEvent } from './types';
import * as ipc from './ipc';
import { useAppStore } from './store/appStore';
import { useAppChrome } from './useAppChrome';
import Sidebar from './components/Sidebar';
import AppsPanel from './components/AppsPanel';
import AppPage from './components/AppPage';
import AddAppPage from './components/AddAppPage';
import ActivityPanel from './components/ActivityPanel';
import SettingsPanel from './components/SettingsPanel';
import Toasts from './components/Toasts';

export default function App() {
  const chrome = useAppChrome();
  const page = useAppStore((s) => s.page);
  const loadApps = useAppStore((s) => s.loadApps);
  const applyStatus = useAppStore((s) => s.applyStatus);
  const apps = useAppStore((s) => s.apps);
  const checks = useAppStore((s) => s.checks);

  // Results stream in per app while a check runs.
  useEffect(() => {
    const unlisten = listen<StatusEvent>('app-status', (e) => applyStatus(e.payload));
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [applyStatus]);

  // Load, then check everything once at start, and open what the command
  // line asked for.
  const setPage = useAppStore((s) => s.setPage);
  useEffect(() => {
    loadApps().then(() => useAppStore.getState().checkStale());
    ipc.startRequest().then((req) => {
      if (req?.kind === 'add') setPage({ kind: 'add', url: req.url });
      else if (req?.kind === 'open') setPage({ kind: 'app', id: req.id });
    });
  }, [loadApps, setPage]);

  // Coming back to the window re-checks what has gone stale meanwhile.
  useEffect(() => {
    const onFocus = () => useAppStore.getState().checkStale();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, []);

  return (
    <div className={chrome.className} style={chrome.style}>
      <Sidebar updates={countUpdates(apps, checks)} />
      <main className="main">
        {page.kind === 'activity' ? (
          <ActivityPanel />
        ) : page.kind === 'settings' ? (
          <SettingsPanel />
        ) : page.kind === 'app' ? (
          <AppPage id={page.id} />
        ) : page.kind === 'add' ? (
          <AddAppPage initialUrl={page.url} />
        ) : (
          <AppsPanel />
        )}
        <Toasts />
      </main>
    </div>
  );
}
