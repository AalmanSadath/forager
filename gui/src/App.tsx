import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { countUpdates } from './appsView';
import type { StatusEvent } from './types';
import { useAppStore } from './store/appStore';
import { useAppChrome } from './useAppChrome';
import Sidebar from './components/Sidebar';
import AppsPanel from './components/AppsPanel';
import ActivityPanel from './components/ActivityPanel';
import SettingsPanel from './components/SettingsPanel';
import Toasts from './components/Toasts';

export default function App() {
  const chrome = useAppChrome();
  const page = useAppStore((s) => s.page);
  const loadApps = useAppStore((s) => s.loadApps);
  const checkApps = useAppStore((s) => s.checkApps);
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

  // Load, then check everything once at start.
  useEffect(() => {
    loadApps().then(() => checkApps());
  }, [loadApps, checkApps]);

  return (
    <div className={chrome.className} style={chrome.style}>
      <Sidebar updates={countUpdates(apps, checks)} />
      <main className="main">
        {page.kind === 'activity' ? <ActivityPanel /> : page.kind === 'settings' ? <SettingsPanel /> : <AppsPanel />}
        <Toasts />
      </main>
    </div>
  );
}
