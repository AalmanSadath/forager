import { describe, expect, it } from 'vitest';
import { accentTokens, contrast, luminance, parseHex } from './accent';

describe('parseHex', () => {
  it('takes both lengths, with or without the hash', () => {
    expect(parseHex('#9141ac')).toEqual([145, 65, 172]);
    expect(parseHex('9141AC')).toEqual([145, 65, 172]);
    expect(parseHex('#abc')).toEqual([170, 187, 204]);
  });

  it('refuses anything that is not a colour', () => {
    for (const bad of ['', '#', 'purple', '#12345', '#gggggg', 'rgb(1,2,3)']) {
      expect(parseHex(bad), bad).toBeNull();
    }
  });
});

describe('luminance', () => {
  it('matches the WCAG endpoints', () => {
    expect(luminance([0, 0, 0])).toBe(0);
    expect(luminance([255, 255, 255])).toBeCloseTo(1, 10);
  });

  it('gives white on black the ratio the spec names', () => {
    expect(contrast([255, 255, 255], [0, 0, 0])).toBeCloseTo(21, 10);
  });
});

describe('accentTokens', () => {
  it('returns null rather than tokens for a colour it cannot read', () => {
    expect(accentTokens('not a colour')).toBeNull();
  });

  /**
   * The reason this exists. A pale accent with white written on it is the
   * failure the built-in palettes avoided by hand-picking --on-accent per
   * theme, which stops being possible once the colour is the user's.
   */
  it('writes dark on a pale accent and light on a deep one', () => {
    expect(accentTokens('#ffe066')?.onAccent).toBe('#0d1117');
    expect(accentTokens('#0b3d91')?.onAccent).toBe('#ffffff');
  });

  it('always clears AA for normal text against its own accent', () => {
    // A spread around the hue circle, including the greens and yellows that a
    // lightness threshold gets wrong.
    const accents = ['#9141ac', '#ffe066', '#3fb950', '#0b3d91', '#ff7b72',
                     '#00bcd4', '#7f7f7f', '#000000', '#ffffff'];
    for (const hex of accents) {
      const t = accentTokens(hex)!;
      const ratio = contrast(parseHex(t.accent)!, parseHex(t.onAccent)!);
      expect(ratio, `${hex} on ${t.onAccent}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  /**
   * Hover moves away from the text drawn on the accent, which is what the
   * built-in palettes do: the dark theme's pale blue #58a6ff hovers lighter to
   * #79c0ff, and the light theme's deep #155397 hovers darker to #0f4478. Both
   * step away from their own foreground, so the label never loses contrast
   * under the cursor.
   */
  it('moves the hover shade away from its own foreground', () => {
    for (const hex of ['#0b3d91', '#ffe066', '#9141ac', '#3fb950']) {
      const t = accentTokens(hex)!;
      const base = luminance(parseHex(t.accent)!);
      const hover = luminance(parseHex(t.accentHover)!);
      const foreground = luminance(parseHex(t.onAccent)!);
      // Away from the foreground: darker when the text is white, lighter when
      // the text is near-black.
      expect(hover, `${hex} hovers the wrong way`).toSatisfy(
        (h: number) => (foreground > base ? h < base : h > base),
      );
    }
  });

  it('never loses contrast under the cursor', () => {
    for (const hex of ['#0b3d91', '#ffe066', '#9141ac', '#3fb950', '#7f7f7f']) {
      const t = accentTokens(hex)!;
      const ratio = contrast(parseHex(t.accentHover)!, parseHex(t.onAccent)!);
      expect(ratio, `${hex} hovered`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it('keeps the accent it was given', () => {
    expect(accentTokens('#9141AC')?.accent).toBe('#9141ac');
  });
});
