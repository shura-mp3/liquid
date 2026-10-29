import { describe, expect, it } from 'vitest';
import {
  MIN_CONTRAST,
  contrast,
  extractPalette,
  fromHsl,
  luminance,
  readableTextOn,
  toCssTriplet,
  toHsl,
  type Rgb,
} from '../src/core/colour';

const WHITE: Rgb = { r: 255, g: 255, b: 255 };
const BLACK: Rgb = { r: 0, g: 0, b: 0 };

/** Build an RGBA buffer from a list of colours, repeated. */
function pixels(colours: Rgb[], repeat = 1): Uint8ClampedArray {
  const out = new Uint8ClampedArray(colours.length * repeat * 4);
  let i = 0;
  for (let n = 0; n < repeat; n += 1) {
    for (const c of colours) {
      out[i] = c.r;
      out[i + 1] = c.g;
      out[i + 2] = c.b;
      out[i + 3] = 255;
      i += 4;
    }
  }
  return out;
}

describe('luminance and contrast', () => {
  it('matches the WCAG endpoints', () => {
    expect(luminance(WHITE)).toBeCloseTo(1, 4);
    expect(luminance(BLACK)).toBeCloseTo(0, 4);
  });

  it('gives the known 21:1 maximum', () => {
    expect(contrast(WHITE, BLACK)).toBeCloseTo(21, 1);
  });

  it('is symmetric', () => {
    const a: Rgb = { r: 12, g: 90, b: 180 };
    expect(contrast(a, WHITE)).toBeCloseTo(contrast(WHITE, a), 10);
  });

  it('is 1 for identical colours', () => {
    expect(contrast(WHITE, WHITE)).toBeCloseTo(1, 10);
  });
});

describe('readableTextOn', () => {
  it('chooses dark text on a light background', () => {
    expect(readableTextOn({ r: 250, g: 248, b: 240 }).colour).toEqual(BLACK);
  });

  it('chooses light text on a dark background', () => {
    expect(readableTextOn({ r: 18, g: 20, b: 34 }).colour).toEqual(WHITE);
  });

  /**
   * The property the album-adaptive theme depends on: there is no artwork colour
   * that can make lyrics fail the contrast threshold. Swept across the whole cube,
   * not spot-checked.
   */
  it('always clears MIN_CONTRAST for every colour in the RGB cube', () => {
    let worst = Infinity;
    let worstColour: Rgb = BLACK;
    for (let r = 0; r <= 255; r += 15) {
      for (let g = 0; g <= 255; g += 15) {
        for (let b = 0; b <= 255; b += 15) {
          const { ratio } = readableTextOn({ r, g, b });
          if (ratio < worst) {
            worst = ratio;
            worstColour = { r, g, b };
          }
        }
      }
    }
    expect(worst, `worst case was ${JSON.stringify(worstColour)}`).toBeGreaterThanOrEqual(
      MIN_CONTRAST,
    );
    // The theoretical floor is ~4.58; anything far above it means the sweep missed.
    expect(worst).toBeLessThan(5.2);
  });
});

describe('hsl round trip', () => {
  it('survives a round trip within rounding error', () => {
    for (const c of [
      { r: 200, g: 40, b: 90 },
      { r: 12, g: 180, b: 160 },
      { r: 255, g: 255, b: 255 },
      { r: 0, g: 0, b: 0 },
      { r: 128, g: 128, b: 128 },
    ]) {
      const { h, s, l } = toHsl(c);
      const back = fromHsl(h, s, l);
      expect(Math.abs(back.r - c.r)).toBeLessThanOrEqual(2);
      expect(Math.abs(back.g - c.g)).toBeLessThanOrEqual(2);
      expect(Math.abs(back.b - c.b)).toBeLessThanOrEqual(2);
    }
  });

  it('handles hue wrap-around', () => {
    expect(fromHsl(360, 1, 0.5)).toEqual(fromHsl(0, 1, 0.5));
    expect(fromHsl(-30, 1, 0.5)).toEqual(fromHsl(330, 1, 0.5));
  });

  it('reports no hue for greys', () => {
    expect(toHsl({ r: 100, g: 100, b: 100 }).s).toBe(0);
  });
});

describe('extractPalette', () => {
  it('finds a saturated dominant colour', () => {
    const p = extractPalette(pixels([{ r: 200, g: 30, b: 60 }], 200), 1);
    expect(p.dominant.r).toBeGreaterThan(p.dominant.g);
    expect(p.dominant.r).toBeGreaterThan(p.dominant.b);
  });

  it('prefers the saturated colour over a dominant near-black border', () => {
    // Most album art has a dark border; naive frequency counting picks it.
    const art = pixels(
      [
        { r: 4, g: 4, b: 6 },
        { r: 4, g: 4, b: 6 },
        { r: 4, g: 4, b: 6 },
        { r: 220, g: 60, b: 160 },
      ],
      60,
    );
    const p = extractPalette(art, 1);
    expect(p.dominant.r + p.dominant.g + p.dominant.b).toBeGreaterThan(120);
  });

  it('returns a readable text colour for the palette it chose', () => {
    const p = extractPalette(pixels([{ r: 240, g: 236, b: 220 }], 100), 1);
    expect(contrast(p.dominant, p.textOnDominant)).toBeGreaterThanOrEqual(MIN_CONTRAST);
    expect(p.contrastRatio).toBeGreaterThanOrEqual(MIN_CONTRAST);
  });

  it('produces an accent distinct from the dominant colour on multi-hue art', () => {
    const p = extractPalette(
      pixels(
        [
          { r: 210, g: 40, b: 40 },
          { r: 40, g: 80, b: 210 },
        ],
        80,
      ),
      1,
    );
    const dh = toHsl(p.dominant).h;
    const ah = toHsl(p.accent).h;
    const delta = Math.min(Math.abs(dh - ah), 360 - Math.abs(dh - ah));
    expect(delta).toBeGreaterThan(20);
  });

  it('still produces an accent for monochrome art rather than duplicating the dominant', () => {
    const p = extractPalette(pixels([{ r: 120, g: 120, b: 122 }], 100), 1);
    expect(p.accent).not.toEqual(p.dominant);
  });

  it('keeps the background darker than the dominant colour', () => {
    const p = extractPalette(pixels([{ r: 180, g: 90, b: 40 }], 100), 1);
    expect(luminance(p.background)).toBeLessThan(luminance(p.dominant));
  });

  it('falls back to a neutral palette for empty input', () => {
    const p = extractPalette(new Uint8ClampedArray(0), 1);
    expect(p.contrastRatio).toBeGreaterThanOrEqual(MIN_CONTRAST);
  });

  it('ignores fully transparent padding', () => {
    const buf = new Uint8ClampedArray(8);
    // Two transparent pixels only.
    const p = extractPalette(buf, 1);
    expect(p.contrastRatio).toBeGreaterThanOrEqual(MIN_CONTRAST);
  });
});

describe('toCssTriplet', () => {
  it('emits the space-separated form the tokens expect', () => {
    expect(toCssTriplet({ r: 1, g: 2, b: 3 })).toBe('1 2 3');
  });
});
