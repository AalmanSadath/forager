import { Component, type ReactNode } from 'react';
import { CHECKS_KEY } from '../../store/appStore';

/**
 * Shows a render error instead of an empty window, with a way out: the
 * likeliest cause is data saved by another version, and that is only a
 * cache.
 */
export default class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="crash">
        <h1>Forager hit a problem</h1>
        <p className="form-error mono">{error.message}</p>
        <p className="form-hint">Clearing the saved check results and reloading usually fixes this. Tracked apps are not affected.</p>
        <button
          className="btn-primary"
          onClick={() => {
            localStorage.removeItem(CHECKS_KEY);
            location.reload();
          }}
        >
          Clear and reload
        </button>
      </div>
    );
  }
}
