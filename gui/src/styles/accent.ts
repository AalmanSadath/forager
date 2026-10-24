// Copied from BifroSSH (src/styles/accent.ts), same author.

/**
 * Turning one chosen colour into the three tokens the stylesheet needs.
 *
 * `--accent` is only a third of it. Every palette also hand-picks an
 * `--on-accent` to write on top of it and an `--accent-hover` a shade along,
 * and those were chosen against a known colour. Once the accent is whatever
 * the user or their desktop says, both have to be derived, or a pale accent
 * gets white text on it and disappears.
 */

/** Parses `#rgb` or `#rrggbb`. Returns null for anything else. */
export function parseHex(hex: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const s = m[1].length === 3 ? m[1].replace(/./g, (c) => c + c) : m[1];
  return [0, 2, 4].map((i) => parseInt(s.slice(i, i + 2), 16)) as [number, number, number];
}

/** The inverse of `parseHex`, clamping each channel into range. */
export function toHex([r, g, b]: [number, number, number]): string {
  const c = (v: number) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, '0');
  return `#${c(r)}${c(g)}${c(b)}`;
}

/** WCAG relative luminance. The same maths the contrast pass used. */
export function luminance([r, g, b]: [number, number, number]): number {
  const channel = (v: number) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

export function contrast(a: [number, number, number], b: [number, number, number]): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** The near-black every palette already uses for text on a light field. */
const INK: [number, number, number] = [13, 17, 23];
const PAPER: [number, number, number] = [255, 255, 255];

/**
 * The three tokens a chosen accent implies.
 *
 * The foreground is whichever of the app's own near-black and white stands out
 * further against the accent, rather than a lightness threshold: a mid green
 * and a mid blue of the same lightness do not want the same text on them.
 *
 * The hover shade moves away from the foreground, so a dark accent lightens
 * and a light one darkens. Moving one direction always would have made a
 * hovered pale accent brighter than the page.
 */
export function accentTokens(hex: string): {
  accent: string;
  accentHover: string;
  onAccent: string;
} | null {
  const rgb = parseHex(hex);
  if (!rgb) return null;

  const onInk = contrast(rgb, INK);
  const onPaper = contrast(rgb, PAPER);
  const foreground = onInk >= onPaper ? INK : PAPER;

  // 12% along, which is what the built-in palettes move between their own
  // accent and its hover.
  const towards = foreground === INK ? PAPER : INK;
  const hover = rgb.map((v, i) => v + (towards[i] - v) * 0.12) as [number, number, number];

  return {
    accent: toHex(rgb),
    accentHover: toHex(hover),
    onAccent: toHex(foreground),
  };
}
