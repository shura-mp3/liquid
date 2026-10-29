/**
 * Generates the LIQUID source icon as a PNG, with no image dependencies.
 *
 * Kept as a script rather than a committed binary so the icon is reproducible and
 * reviewable. Run `node scripts/make-icon.mjs` then `npx tauri icon` to produce the
 * platform icon set.
 */

import { deflateSync } from 'node:zlib';
import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SIZE = 1024;
const here = dirname(fileURLToPath(import.meta.url));
const out = resolve(here, '../src-tauri/icons/source.png');

// --- PNG writing -----------------------------------------------------------

const CRC_TABLE = (() => {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
})();

function crc32(buf) {
  let c = -1;
  for (let i = 0; i < buf.length; i += 1) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function png(width, height, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // colour type: RGBA
  // 10..12 = compression, filter, interlace: all 0

  // Filter type 0 (none) in front of every scanline.
  const stride = width * 4;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y += 1) {
    raw[y * (stride + 1)] = 0;
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

// --- the artwork -----------------------------------------------------------

const lerp = (a, b, t) => a + (b - a) * t;
const clamp01 = (t) => (t < 0 ? 0 : t > 1 ? 1 : t);
const smooth = (edge0, edge1, x) => {
  const t = clamp01((x - edge0) / (edge1 - edge0));
  return t * t * (3 - 2 * t);
};

/** A rounded-square glass tile with a diagonal gradient, specular sweep and rim. */
function render() {
  const px = Buffer.alloc(SIZE * SIZE * 4);
  const c = SIZE / 2;
  const half = SIZE * 0.4;
  const radius = SIZE * 0.14;

  // Liquid Midnight palette: deep indigo into cyan, with a violet accent.
  const deep = [28, 32, 68];
  const mid = [58, 94, 180];
  const bright = [92, 214, 230];
  const accent = [148, 108, 232];

  for (let y = 0; y < SIZE; y += 1) {
    for (let x = 0; x < SIZE; x += 1) {
      const i = (y * SIZE + x) * 4;

      // Signed distance to a rounded square, for a clean antialiased edge.
      const dx = Math.abs(x - c) - (half - radius);
      const dy = Math.abs(y - c) - (half - radius);
      const outside =
        Math.hypot(Math.max(dx, 0), Math.max(dy, 0)) + Math.min(Math.max(dx, dy), 0) - radius;
      const coverage = 1 - smooth(-1.5, 1.5, outside);
      if (coverage <= 0) continue;

      // Diagonal gradient across the tile.
      const t = clamp01(((x - c) / (2 * half) + (y - c) / (2 * half) + 1) / 2);
      let r = lerp(lerp(deep[0], mid[0], smooth(0, 0.65, t)), bright[0], smooth(0.55, 1, t));
      let g = lerp(lerp(deep[1], mid[1], smooth(0, 0.65, t)), bright[1], smooth(0.55, 1, t));
      let b = lerp(lerp(deep[2], mid[2], smooth(0, 0.65, t)), bright[2], smooth(0.55, 1, t));

      // Violet bloom in the lower-left, like album tint bleeding through.
      const bloom = smooth(SIZE * 0.55, 0, Math.hypot(x - SIZE * 0.3, y - SIZE * 0.72)) * 0.45;
      r = lerp(r, accent[0], bloom);
      g = lerp(g, accent[1], bloom);
      b = lerp(b, accent[2], bloom);

      // A soft liquid wave through the middle, the visualiser motif.
      const waveY = c + Math.sin((x / SIZE) * Math.PI * 3) * SIZE * 0.055;
      const wave = smooth(SIZE * 0.07, 0, Math.abs(y - waveY)) * 0.55;
      r = lerp(r, 255, wave * 0.85);
      g = lerp(g, 255, wave * 0.9);
      b = lerp(b, 255, wave * 0.95);

      // Specular sweep: a bright diagonal band in the upper third.
      const sweep = smooth(SIZE * 0.1, 0, Math.abs(x * 0.6 + y - SIZE * 0.42)) * 0.3;
      r = lerp(r, 255, sweep);
      g = lerp(g, 255, sweep);
      b = lerp(b, 255, sweep);

      // Glass rim: brighten the last couple of pixels before the edge.
      const rim = smooth(-9, -1.5, outside) * 0.55;
      r = lerp(r, 236, rim);
      g = lerp(g, 248, rim);
      b = lerp(b, 255, rim);

      px[i] = Math.round(clamp01(r / 255) * 255);
      px[i + 1] = Math.round(clamp01(g / 255) * 255);
      px[i + 2] = Math.round(clamp01(b / 255) * 255);
      px[i + 3] = Math.round(coverage * 255);
    }
  }
  return px;
}

mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, png(SIZE, SIZE, render()));
console.log(`wrote ${out} (${SIZE}x${SIZE})`);
