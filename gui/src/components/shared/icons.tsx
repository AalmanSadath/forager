/**
 * Icons drawn in more than one place. Line icons on a 24 grid, stroked in
 * currentColor, as BifroSSH draws its own.
 */
import type { ReactNode } from 'react';
import type { Format, Forge } from '../../types';

function Svg({ size = 20, width = 2.2, children }: { size?: number; width?: number; children: ReactNode }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={width} strokeLinecap="round" strokeLinejoin="round">
      {children}
    </svg>
  );
}

/** Forager's mark: a tray, an arrow arriving in it, and a leaf. */
export function BrandMark({ size = 28 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 512 512" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" style={{ flexShrink: 0 }}>
      <path d="M 110 300 L 110 382 Q 110 410 138 410 L 374 410 Q 402 410 402 382 L 402 300" strokeWidth="30" />
      <path d="M 256 120 L 256 324" strokeWidth="30" />
      <path d="M 186 254 L 256 324 L 326 254" strokeWidth="30" />
      <path d="M 256 186 Q 268 108 352 96 Q 344 176 256 186 Z" strokeWidth="24" />
    </svg>
  );
}

export const AppsIcon = () => (
  <Svg>
    <rect x="3" y="3" width="7" height="7" rx="1.5" />
    <rect x="14" y="3" width="7" height="7" rx="1.5" />
    <rect x="3" y="14" width="7" height="7" rx="1.5" />
    <rect x="14" y="14" width="7" height="7" rx="1.5" />
  </Svg>
);

export const ActivityIcon = () => (
  <Svg>
    <path d="M3 12h4l3 8 4-16 3 8h4" />
  </Svg>
);

export const SettingsIcon = () => (
  <Svg>
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z" />
  </Svg>
);

export const BackIcon = () => (
  <Svg size={18}>
    <polyline points="15,18 9,12 15,6" />
  </Svg>
);

export const ExternalIcon = ({ size = 13 }: { size?: number }) => (
  <Svg size={size} width={2}>
    <path d="M14 4h6v6" />
    <path d="M20 4l-9 9" />
    <path d="M18 14v5a1 1 0 01-1 1H5a1 1 0 01-1-1V7a1 1 0 011-1h5" />
  </Svg>
);

/** What kind of thing an install is, for the card's icon tile. */
export function FormatIcon({ format, size = 22 }: { format: Format | null; size?: number }) {
  switch (format) {
    case 'deb':
    case 'rpm':
    case 'pacman':
      // A package: a box with its tape.
      return (
        <Svg size={size} width={1.8}>
          <path d="M12 2l9 5v10l-9 5-9-5V7l9-5z" />
          <path d="M3 7l9 5 9-5" />
          <path d="M12 12v10" />
          <path d="M7.5 4.5l9 5" />
        </Svg>
      );
    case 'flatpak':
      // A sealed cube: sandboxed.
      return (
        <Svg size={size} width={1.8}>
          <path d="M12 2l9 5v10l-9 5-9-5V7l9-5z" />
          <path d="M3 7l9 5 9-5" />
          <path d="M12 12v10" />
          <circle cx="12" cy="12" r="2.5" />
        </Svg>
      );
    case 'appimage':
      // One file that runs.
      return (
        <Svg size={size} width={1.8}>
          <path d="M14 3H6a2 2 0 00-2 2v14a2 2 0 002 2h12a2 2 0 002-2V9z" />
          <path d="M14 3v6h6" />
          <path d="M10 12l4 2.5-4 2.5z" />
        </Svg>
      );
    case 'tarball':
      // An archive box.
      return (
        <Svg size={size} width={1.8}>
          <rect x="3" y="4" width="18" height="5" rx="1" />
          <path d="M5 9v10a1 1 0 001 1h12a1 1 0 001-1V9" />
          <path d="M10 13h4" />
        </Svg>
      );
    default:
      // Not installed yet: an empty tray.
      return (
        <Svg size={size} width={1.8}>
          <path d="M4 14v4a2 2 0 002 2h12a2 2 0 002-2v-4" />
          <path d="M12 4v10" />
          <path d="M8 10l4 4 4-4" />
        </Svg>
      );
  }
}

export const FORGE_NAMES: Record<Forge, string> = {
  github: 'GitHub',
  gitlab: 'GitLab',
  forgejo: 'Forgejo',
};

export const FORMAT_NAMES: Record<Format, string> = {
  deb: 'deb',
  rpm: 'rpm',
  pacman: 'Arch',
  appimage: 'AppImage',
  tarball: 'Tarball',
  flatpak: 'Flatpak',
};
