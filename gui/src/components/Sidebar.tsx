import { useState } from 'react';
import { useAppStore } from '../store/appStore';
import { ActivityIcon, AppsIcon, BrandMark, SettingsIcon } from './shared/icons';

/**
 * The left rail, as in BifroSSH: brand, the pages you work in, and settings
 * fenced off at the foot.
 */
export default function Sidebar({ updates }: { updates: number }) {
  const page = useAppStore((s) => s.page);
  const setPage = useAppStore((s) => s.setPage);
  const [collapsed, setCollapsed] = useState(false);

  // An app's page and the add page belong to Apps.
  const section = page.kind === 'app' || page.kind === 'add' ? 'apps' : page.kind;

  const nav = (kind: 'apps' | 'activity' | 'settings', label: string, icon: React.ReactNode, badge?: number) => (
    <button
      className={`nav-btn${section === kind ? ' active' : ''}`}
      onClick={() => setPage({ kind })}
      title={collapsed ? label : undefined}
    >
      <span className="nav-icon">{icon}</span>
      {!collapsed && label}
      {!!badge && <span className="nav-badge" title={`${badge} update${badge === 1 ? '' : 's'} available`}>{badge}</span>}
    </button>
  );

  return (
    <aside className={`sidebar${collapsed ? ' sidebar-collapsed' : ''}`}>
      <div className="sidebar-top">
        {!collapsed && (
          <div className="sidebar-brand">
            <BrandMark />
            <span className="sidebar-brand-text">Forager</span>
          </div>
        )}
        <button
          className="sidebar-collapse-btn"
          onClick={() => setCollapsed((c) => !c)}
          title={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
        >
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
            {collapsed ? <polyline points="9,18 15,12 9,6" /> : <polyline points="15,18 9,12 15,6" />}
          </svg>
        </button>
      </div>
      <nav className="sidebar-nav">
        {nav('apps', 'Apps', <AppsIcon />, updates)}
        {nav('activity', 'Activity', <ActivityIcon />)}
      </nav>
      <nav className="sidebar-nav-bottom">{nav('settings', 'Settings', <SettingsIcon />)}</nav>
    </aside>
  );
}
