import { useEffect } from 'react';
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

  useEffect(() => {
    loadApps();
  }, [loadApps]);

  return (
    <div className={chrome.className} style={chrome.style}>
      <Sidebar updates={0} />
      <main className="main">
        {page.kind === 'activity' ? <ActivityPanel /> : page.kind === 'settings' ? <SettingsPanel /> : <AppsPanel />}
        <Toasts />
      </main>
    </div>
  );
}
