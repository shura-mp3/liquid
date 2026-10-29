/**
 * Album artwork palette extraction with a contrast guarantee.
 *
 * Album-adaptive theming is the fastest way to make a lyrics HUD unreadable: extract
 * a muddy brown from a dark cover, tint the glass with it, and the white text
 * disappears. So extraction here is not just "find the dominant colour" — every
 * palette is checked against WCAG contrast and adjusted until the text is legible.
 * A pretty tint that cannot be read is a bug, not a style.
 */

export interface Rgb {
  r: number;
  g: number;
  b: number;
}

export interface Palette {
  dominant: Rgb;
  accent: Rgb;
  /** Background base for gradients, darkened/lightened from the dominant colour. */
  background: Rgb;
  /** Text colour guaranteed to meet `MIN_CONTRAST` against `dominant`. */
  textOnDominant: Rgb;
  /** Measured contrast ratio actually achieved, for diagnostics. */
  contrastRatio: number;
}

/** WCAG AA for large text. Lyrics are large, and this is a HUD, not a document. */
export const MIN_CONTRAST = 4.5;

const WHITE: Rgb = { r: 255, g: 255, b: 255 };
const BLACK: Rgb = { r: 0, g: 0, b: 0 };

const clamp255 = (n: number) => (n < 0 ? 0 : n > 255 ? 255 : Math.round(n));

/** Relative luminance per WCAG 2.x. */
export function luminance({ r, g, b }: Rgb): number {
  const channel = (v: number) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

/** WCAG contrast ratio, always >= 1. */
export function contrast(a: Rgb, b: Rgb): number {
  const la = luminance(a);
  const lb = luminance(b);
  const [hi, lo] = la > lb ? [la, lb] : [lb, la];
  return (hi + 0.05) / (lo + 0.05);
}

export function toHsl({ r, g, b }: Rgb): { h: number; s: number; l: number } {
  const rn = r / 255;
  const gn = g / 255;
  const bn = b / 255;
  const max = Math.max(rn, gn, bn);
  const min = Math.min(rn, gn, bn);
  const l = (max + min) / 2;
  const d = max - min;

  if (d === 0) return { h: 0, s: 0, l };

  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h: number;
  if (max === rn) h = ((gn - bn) / d + (gn < bn ? 6 : 0)) / 6;
  else if (max === gn) h = ((bn - rn) / d + 2) / 6;
  else h = ((rn - gn) / d + 4) / 6;
  return { h: h * 360, s, l };
}

export function fromHsl(h: number, s: number, l: number): Rgb {
  const hn = (((h % 360) + 360) % 360) / 360;
  if (s === 0) {
    const v = clamp255(l * 255);
    return { r: v, g: v, b: v };
  }
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const toChannel = (t: number) => {
    let tn = t;
    if (tn < 0) tn += 1;
    if (tn > 1) tn -= 1;
    if (tn < 1 / 6) return p + (q - p) * 6 * tn;
    if (tn < 1 / 2) return q;
    if (tn < 2 / 3) return p + (q - p) * (2 / 3 - tn) * 6;
    return p;
  };
  return {
    r: clamp255(toChannel(hn + 1 / 3) * 255),
    g: clamp255(toChannel(hn) * 255),
    b: clamp255(toChannel(hn - 1 / 3) * 255),
  };
}

/**
 * Pick whichever of black or white reads better on this background.
 *
 * No fallback is needed, and that is a provable property rather than an assumption.
 * The two ratios are `1.05/(L+0.05)` for white and `(L+0.05)/0.05` for black; the
 * worse of the two is minimised where they are equal, at `L ≈ 0.179`, where both
 * equal about **4.58**. So `max(white, black)` is never below 4.58 for any colour,
 * which always clears `MIN_CONTRAST` of 4.5. `contrastRatio` is still returned so a
 * future threshold change cannot quietly invalidate this.
 */
export function readableTextOn(background: Rgb): { colour: Rgb; ratio: number } {
  const white = contrast(background, WHITE);
  const black = contrast(background, BLACK);
  return white >= black
    ? { colour: WHITE, ratio: white }
    : { colour: BLACK, ratio: black };
}

/**
 * Quantise pixels into a coarse colour cube and score the buckets.
 *
 * Plain "most frequent colour" loses to the near-black or near-white border that
 * most album art has, so very dark, very light and very desaturated buckets are
 * penalised rather than excluded outright — excluded entirely would fail on art that
 * genuinely is monochrome.
 */
export function extractPalette(pixels: Uint8ClampedArray, stride = 4): Palette {
  const BUCKETS = 6; // per channel
  const counts = new Map<number, { count: number; r: number; g: number; b: number }>();

  for (let i = 0; i + 3 < pixels.length; i += stride * 4) {
    const a = pixels[i + 3]!;
    if (a < 128) continue; // ignore transparent padding
    const r = pixels[i]!;
    const g = pixels[i + 1]!;
    const b = pixels[i + 2]!;
    const key =
      (Math.floor((r / 256) * BUCKETS) * BUCKETS + Math.floor((g / 256) * BUCKETS)) * BUCKETS +
      Math.floor((b / 256) * BUCKETS);
    const entry = counts.get(key);
    if (entry) {
      entry.count += 1;
      entry.r += r;
      entry.g += g;
      entry.b += b;
    } else {
      counts.set(key, { count: 1, r, g, b });
    }
  }

  if (counts.size === 0) {
    return neutralPalette();
  }

  const scored = [...counts.values()].map((e) => {
    const colour: Rgb = {
      r: clamp255(e.r / e.count),
      g: clamp255(e.g / e.count),
      b: clamp255(e.b / e.count),
    };
    const { s, l } = toHsl(colour);
    // Favour mid-lightness, reasonably saturated colours: they read as "the colour
    // of the album" rather than as its border.
    const lightnessScore = 1 - Math.abs(l - 0.5) * 1.6;
    const saturationScore = 0.25 + s * 0.75;
    return {
      colour,
      weight: e.count * Math.max(0.05, lightnessScore) * saturationScore,
      count: e.count,
      s,
      l,
    };
  });

  scored.sort((a, b) => b.weight - a.weight);
  const dominant = scored[0]!.colour;

  // Accent: the highest-weighted bucket whose hue differs usefully from the
  // dominant one, falling back to a hue rotation when the art is monochrome.
  const domHsl = toHsl(dominant);
  const distinct = scored.find((c) => {
    const d = Math.abs(toHsl(c.colour).h - domHsl.h);
    const hueDelta = Math.min(d, 360 - d);
    return hueDelta > 25 && c.weight > scored[0]!.weight * 0.12;
  });
  const accent = distinct
    ? distinct.colour
    : fromHsl(domHsl.h + 32, Math.min(1, domHsl.s + 0.15), Math.min(0.72, domHsl.l + 0.12));

  // Background: a deep version of the dominant hue, so gradients stay in family.
  const background = fromHsl(domHsl.h, Math.min(0.6, domHsl.s), Math.min(0.16, domHsl.l * 0.4));

  const { colour: textOnDominant, ratio } = readableTextOn(dominant);

  return { dominant, accent, background, textOnDominant, contrastRatio: ratio };
}

function neutralPalette(): Palette {
  const dominant: Rgb = { r: 88, g: 118, b: 210 };
  const { colour, ratio } = readableTextOn(dominant);
  return {
    dominant,
    accent: { r: 148, g: 108, b: 232 },
    background: { r: 10, g: 12, b: 24 },
    textOnDominant: colour,
    contrastRatio: ratio,
  };
}

export const NEUTRAL_PALETTE = neutralPalette();

/** `"r g b"`, the space-separated form the CSS tokens expect. */
export function toCssTriplet({ r, g, b }: Rgb): string {
  return `${r} ${g} ${b}`;
}
