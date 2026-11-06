import { pageTitle } from '../appsView';
import { useAppStore } from '../store/appStore';
import WindowControls from './WindowControls';

/**
 * The bar across the top of the page area: the page's title, and the window
 * buttons when the app draws its own frame. Empty space in it, the title
 * included, drags the window, and a double click there maximizes it.
 */
export default function TopBar() {
  const page = useAppStore((s) => s.page);
  const apps = useAppStore((s) => s.apps);
  return (
    <header className="top-bar" data-tauri-drag-region>
      <h1 className="top-bar-title" data-tauri-drag-region>
        {pageTitle(page, apps)}
      </h1>
      <WindowControls />
    </header>
  );
}
