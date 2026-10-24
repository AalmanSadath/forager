import { useAppStore } from '../../store/appStore';
import { resolveAccent, resolveAppTheme, THEME_ACCENTS } from '../../store/theme';
import type { AppTheme } from '../../types';

/** System first: it is the one that defers rather than decides. */
const APP_THEMES: { value: AppTheme; label: string }[] = [
  { value: 'system', label: 'System' },
  { value: 'dark', label: 'Dark' },
  { value: 'light', label: 'Light' },
  { value: 'amoled', label: 'AMOLED' },
];

/** Theme and accent, as BifroSSH offers them. */
export default function AppearanceSection() {
  const settings = useAppStore((s) => s.settings);
  const system = useAppStore((s) => s.systemAppearance);
  const patch = useAppStore((s) => s.patchSettings);
  const accent = resolveAccent(settings, system) ?? THEME_ACCENTS[resolveAppTheme(settings.app_theme, system)];

  return (
    <section className="panel-section">
      <h3>Appearance</h3>
      <div className="form-group">
        <label>Theme</label>
        <div className="toggle-row" style={{ maxWidth: 320 }}>
          {APP_THEMES.map(({ value, label }) => (
            <button
              key={value}
              type="button"
              className={`toggle-btn${settings.app_theme === value ? ' active' : ''}`}
              onClick={() => patch({ app_theme: value })}
            >
              {label}
            </button>
          ))}
        </div>
        {settings.app_theme === 'system' && (
          <p className="form-hint">
            Following the system theme, which is set to {system.color_scheme === 'dark' ? 'dark' : 'light'}.
          </p>
        )}
      </div>
      <div className="form-group">
        <label htmlFor="accent">Accent colour</label>
        <div className="accent-row">
          <input id="accent" type="color" value={accent} onChange={(e) => patch({ accent_color: e.target.value })} />
          <span className="mono">{accent}</span>
          {settings.accent_color !== null && (
            <button type="button" className="btn-secondary btn-sm" onClick={() => patch({ accent_color: null })}>
              Use system accent
            </button>
          )}
        </div>
        <p className="form-hint">
          {settings.accent_color !== null
            ? 'Your own colour. Reset it to follow the system again.'
            : system.accent
              ? 'Following the system accent, and changes with it.'
              : 'The system exposes no accent, so the theme’s own is used.'}
        </p>
      </div>
    </section>
  );
}
