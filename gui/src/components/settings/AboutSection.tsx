import { useEffect, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import * as ipc from '../../ipc';
import type { AboutInfo } from '../../types';
import { BrandMark, ExternalIcon } from '../shared/icons';

const PROJECT = 'https://github.com/AalmanSadath/forager';

/** Version, and where Forager keeps everything it writes. */
export default function AboutSection() {
  const [info, setInfo] = useState<AboutInfo | null>(null);
  useEffect(() => {
    ipc.aboutInfo().then(setInfo);
  }, []);
  if (!info) return null;
  const rows: [string, string | null][] = [
    ['Tracked apps', info.config],
    ['Tokens', info.env_file],
    ['History', info.history],
    ['Cache', info.cache],
    ['Signing keys', info.keys],
    ['AppImages', info.apps_dir],
    ['Tarballs', info.opt_dir],
  ];
  return (
    <section className="panel-section">
      <h3>About</h3>
      <div className="about-head">
        <BrandMark size={40} />
        <div>
          <div className="about-name">Forager {info.version}</div>
          <button className="link-btn" onClick={() => openUrl(PROJECT).catch(() => {})}>
            {PROJECT.replace('https://', '')} <ExternalIcon />
          </button>
        </div>
      </div>
      <dl className="facts">
        {rows
          .filter(([, v]) => v)
          .map(([k, v]) => (
            <div key={k} style={{ display: 'contents' }}>
              <dt>{k}</dt>
              <dd className="mono">{v}</dd>
            </div>
          ))}
      </dl>
    </section>
  );
}
