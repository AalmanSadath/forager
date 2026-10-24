import ReactDOM from 'react-dom/client';
import App from './App';
import { useAppStore } from './store/appStore';
import './index.css';

document.addEventListener('contextmenu', (e) => e.preventDefault());

// A throw in an event handler or an unawaited promise never reaches React, so
// it is shown rather than lost in a console nobody sees.
window.addEventListener('unhandledrejection', (e) => {
  const reason: unknown = e.reason;
  useAppStore.getState().toast('error', reason instanceof Error ? reason.message : String(reason));
});

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(<App />);
