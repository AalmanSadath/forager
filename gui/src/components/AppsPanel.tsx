import { useAppStore } from '../store/appStore';

export default function AppsPanel() {
  const apps = useAppStore((s) => s.apps);
  return (
    <div className="panel">
      <h1 className="panel-title">Apps</h1>
      <p className="list-empty">{apps.length} tracked.</p>
    </div>
  );
}
