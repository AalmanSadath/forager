import { pageTitle } from '../appsView';
import { useAppStore } from '../store/appStore';

/** The bar across the top of the page area, with the page's title. */
export default function TopBar() {
  const page = useAppStore((s) => s.page);
  const apps = useAppStore((s) => s.apps);
  return (
    <header className="top-bar">
      <h1 className="top-bar-title">{pageTitle(page, apps)}</h1>
    </header>
  );
}
