import type { CardModel } from '../appsView';
import type { AppView } from '../types';
import { FormatIcon, FORMAT_NAMES } from './shared/icons';

const ACTION_LABELS = {
  update: 'Update',
  install: 'Install',
  reinstall: 'Reinstall',
  switch: 'Switch',
  view: 'Release page',
} as const;

/**
 * One app in the grid, laid out like BifroSSH's host card: icon tile, status
 * dot and name with tags, and a monospace line under it.
 */
export default function AppCard({
  app,
  model,
  onOpen,
  onAction,
  progress,
}: {
  app: AppView;
  model: CardModel;
  onOpen: () => void;
  onAction?: () => void;
  /** 0..1 while an update runs. */
  progress?: number | null;
}) {
  return (
    <div
      className="app-card"
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onOpen();
        }
      }}
      title={model.label}
      // Name first, so a screen reader says which app before its state.
      aria-label={`${app.id}, ${model.label}, ${model.versions}`}
    >
      <div className="app-card-icon">
        <FormatIcon format={model.format} />
      </div>
      <div className="app-card-info">
        <div className="app-card-name-row">
          <span className={`dot dot-${model.dot}`} />
          <span className="card-title">{app.id}</span>
          <span className="app-card-tags">
            {model.formats.map((f) => (
              <span key={f} className="tag">{FORMAT_NAMES[f]}</span>
            ))}
            {app.signature && <span className="tag tag-accent">signed</span>}
          </span>
        </div>
        <div className="card-sub">{model.versions}</div>
      </div>
      {model.action && onAction && (
        <button
          className="btn-primary btn-sm app-card-action"
          onClick={(e) => {
            e.stopPropagation();
            onAction();
          }}
        >
          {ACTION_LABELS[model.action]}
        </button>
      )}
      {progress != null && (
        <div className="app-card-progress" style={{ width: `${Math.round(progress * 100)}%` }} />
      )}
    </div>
  );
}
