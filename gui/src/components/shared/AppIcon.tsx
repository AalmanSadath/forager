import { useEffect, useState } from 'react';
import * as ipc from '../../ipc';
import type { AppView, Format } from '../../types';
import { FormatIcon } from './icons';

// One lookup per install state: the icon only changes when an install does.
const icons = new Map<string, Promise<string | null>>();

function iconOf(app: AppView): Promise<string | null> {
  const key = `${app.id}|${app.installs.map((i) => `${i.format}:${i.tag}`).join(',')}`;
  let icon = icons.get(key);
  if (!icon) {
    icon = app.installs.length > 0 ? ipc.appIcon(app.id).catch(() => null) : Promise.resolve(null);
    icons.set(key, icon);
  }
  return icon;
}

/** The app's own icon when an install has one, else its format's. */
export default function AppIcon({ app, format, size }: { app: AppView; format: Format | null; size?: number }) {
  const [url, setUrl] = useState<string | null>(null);
  const [broken, setBroken] = useState(false);

  useEffect(() => {
    let live = true;
    iconOf(app).then((icon) => {
      if (live) {
        setUrl(icon);
        setBroken(false);
      }
    });
    return () => {
      live = false;
    };
  }, [app]);

  if (!url || broken) return <FormatIcon format={format} size={size} />;
  // A little bigger than the format glyph, which has padding of its own.
  const side = (size ?? 22) + 10;
  return <img className="app-icon" src={url} alt="" width={side} height={side} onError={() => setBroken(true)} />;
}
