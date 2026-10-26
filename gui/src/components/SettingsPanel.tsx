import { useState } from 'react';
import AppearanceSection from './settings/AppearanceSection';
import BackgroundSection from './settings/BackgroundSection';
import TokensSection from './settings/TokensSection';
import AppsSection from './settings/AppsSection';
import SystemSection from './settings/SystemSection';
import AboutSection from './settings/AboutSection';

const SECTIONS = [
  { key: 'appearance', label: 'Appearance', body: <AppearanceSection /> },
  { key: 'background', label: 'Background', body: <BackgroundSection /> },
  { key: 'tokens', label: 'Tokens', body: <TokensSection /> },
  { key: 'apps', label: 'Apps', body: <AppsSection /> },
  { key: 'system', label: 'System check', body: <SystemSection /> },
  { key: 'about', label: 'About', body: <AboutSection /> },
] as const;

/** Settings with BifroSSH's category rail. */
export default function SettingsPanel() {
  const [section, setSection] = useState<(typeof SECTIONS)[number]['key']>('appearance');
  return (
    <div className="panel settings-panel">
      <h1 className="panel-title">Settings</h1>
      <div className="settings-layout">
        <nav className="settings-rail">
          {SECTIONS.map(({ key, label }) => (
            <button key={key} className={`settings-rail-item${section === key ? ' active' : ''}`} onClick={() => setSection(key)}>
              {label}
            </button>
          ))}
        </nav>
        <div className="settings-body">{SECTIONS.find((s) => s.key === section)!.body}</div>
      </div>
    </div>
  );
}
