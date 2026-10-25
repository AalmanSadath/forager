import { useCallback, useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { countUpdates } from './appsView';
import type { ProgressEvent, StartRequest, StatusEvent } from './types';
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
import UpdateModal from './components/UpdateModal';

export default function App() {
  const chrome = useAppChrome();
  const page = useAppStore((s) => s.page);
  const loadApps = useAppStore((s) => s.loadApps);
  const applyStatus = useAppStore((s) => s.applyStatus);
  const apps = useAppStore((s) => s.apps);
  const checks = useAppStore((s) => s.checks);

  const updating = useAppStore((s) => s.updating);
  const updateTag = useAppStore((s) => s.updateTag);
  const applyProgress = useAppStore((s) => s.applyProgress);
  useEffect(() => {
    const unlisten = listen<ProgressEvent>('update-progress', (e) => applyProgress(e.payload));
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [applyProgress]);

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
  const openRequest = useCallback(
    (req: StartRequest) => {
      if (req.kind === 'add') setPage({ kind: 'add', url: req.url, link: req.link ?? undefined });
      else setPage({ kind: 'app', id: req.id });
    },
    [setPage],
  );
  useEffect(() => {
    useAppStore.getState().loadSettings();
    loadApps().then(() => useAppStore.getState().checkStale());
    ipc.startRequest().then((req) => req && openRequest(req));
  }, [loadApps, setPage, openRequest]);

  // A link clicked, or `forager-gui add`, while the window is open.
  useEffect(() => {
    const unlisten = listen<StartRequest>('start-request', (e) => openRequest(e.payload));
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [openRequest]);

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
          <AddAppPage key={JSON.stringify(page)} initialUrl={page.url} link={page.link} />
        ) : (
          <AppsPanel />
        )}
        {updating && <UpdateModal key={`${updating.join(',')}@${updateTag ?? ''}`} ids={updating} tag={updateTag} />}
        <Toasts />
      </main>
    </div>
  );
}
