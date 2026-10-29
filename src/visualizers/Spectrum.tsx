/**
 * WebGL2 audio visualiser.
 *
 * One canvas, several modes, all driven by real FFT output from the capture engine.
 * The band and waveform data goes to the GPU as a texture and the shader reads it,
 * so a 256-band spectrum costs one draw call rather than 256 DOM nodes.
 *
 * WebGL2 is the baseline — WebView2 on Windows 10 has it universally — and the
 * component falls back to a plain message rather than a blank box if context
 * creation fails, which happens with some remote-desktop and software-rendering
 * setups.
 *
 * When the signal is silent the loop still runs but the data is zeros, so the
 * display settles to stillness instead of freezing on the last loud frame.
 */

import { useEffect, useRef, useState } from 'react';

import type { AudioReader } from '../hooks/useAudio';

export type VisualiserMode = 'bars' | 'mirrored' | 'circular' | 'waveform' | 'particles';

export const VISUALISER_MODES: Array<{ id: VisualiserMode; label: string }> = [
  { id: 'bars', label: 'Spectrum bars' },
  { id: 'mirrored', label: 'Mirrored spectrum' },
  { id: 'circular', label: 'Circular spectrum' },
  { id: 'waveform', label: 'Waveform' },
  { id: 'particles', label: 'Bass particles' },
];

const VERTEX = `#version 300 es
precision highp float;
// Fullscreen triangle; no vertex buffer needed.
void main() {
  vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`;

const FRAGMENT = `#version 300 es
precision highp float;

uniform sampler2D uData;   // row 0: bands, row 1: waveform
uniform vec2  uResolution;
uniform float uTime;
uniform int   uMode;
uniform float uBass;
uniform float uBeat;       // decaying pulse, 1 at the hit
uniform vec3  uAccent;
uniform vec3  uAccentStrong;
uniform float uBandCount;
uniform float uWaveCount;

out vec4 fragColour;

float band(float x) {
  // Clamp to the texel centres so edge bands are not half-sampled.
  float u = clamp(x, 0.5 / uBandCount, 1.0 - 0.5 / uBandCount);
  return texture(uData, vec2(u, 0.25)).r;
}

float wave(float x) {
  float u = clamp(x, 0.5 / uWaveCount, 1.0 - 0.5 / uWaveCount);
  return texture(uData, vec2(u, 0.75)).r * 2.0 - 1.0;
}

// Soft vertical bar with a glowing cap.
vec3 barColour(float value, float y, float x) {
  float lit = step(y, value);
  float cap = smoothstep(value - 0.03, value, y) * step(y, value + 0.01);
  vec3 base = mix(uAccent, uAccentStrong, clamp(value * 1.3, 0.0, 1.0));
  return base * lit + uAccentStrong * cap * 1.6;
}

void main() {
  vec2 uv = gl_FragCoord.xy / uResolution;
  vec3 colour = vec3(0.0);
  float alpha = 0.0;

  if (uMode == 0) {               // bars
    float gap = 0.18;
    float slot = fract(uv.x * uBandCount);
    float inside = step(gap * 0.5, slot) * step(slot, 1.0 - gap * 0.5);
    float v = band(floor(uv.x * uBandCount) / uBandCount + 0.5 / uBandCount);
    colour = barColour(v, uv.y, uv.x) * inside;
    alpha = step(uv.y, v) * inside;

  } else if (uMode == 1) {        // mirrored
    float y = abs(uv.y - 0.5) * 2.0;
    float slot = fract(uv.x * uBandCount);
    float inside = step(0.09, slot) * step(slot, 0.91);
    float v = band(floor(uv.x * uBandCount) / uBandCount + 0.5 / uBandCount);
    colour = barColour(v, y, uv.x) * inside;
    alpha = step(y, v) * inside;

  } else if (uMode == 2) {        // circular
    vec2 p = (gl_FragCoord.xy - 0.5 * uResolution) / min(uResolution.x, uResolution.y);
    float r = length(p);
    float a = atan(p.y, p.x) / 6.2831853 + 0.5;
    float v = band(a);
    // Radius breathes with the bass so the ring feels driven, not decorative.
    float inner = 0.22 + uBass * 0.05 + uBeat * 0.02;
    float outer = inner + v * 0.26;
    float ring = smoothstep(inner - 0.004, inner, r) * (1.0 - smoothstep(outer, outer + 0.006, r));
    colour = mix(uAccent, uAccentStrong, v) * ring;
    alpha = ring;

  } else if (uMode == 3) {        // waveform
    float v = wave(uv.x);
    float y = (v * 0.42) + 0.5;
    float d = abs(uv.y - y);
    float line = 1.0 - smoothstep(0.0, 0.012, d);
    float glow = (1.0 - smoothstep(0.0, 0.07, d)) * 0.22;
    colour = mix(uAccent, uAccentStrong, abs(v)) * (line + glow);
    alpha = line + glow;

  } else {                        // particles
    // Cheap GPU particles: a hash grid of points pushed outward by bass. No
    // per-particle state on the CPU, so the count costs nothing to raise.
    vec2 p = (gl_FragCoord.xy - 0.5 * uResolution) / min(uResolution.x, uResolution.y);
    for (int i = 0; i < 48; i++) {
      float fi = float(i);
      float seed = fract(sin(fi * 127.1) * 43758.5453);
      float ang = seed * 6.2831853 + uTime * (0.08 + seed * 0.12);
      // Each particle rides its own band, so the motion tracks the spectrum.
      float bv = band(fract(seed * 1.7));
      float rad = 0.10 + fract(seed * 7.3) * 0.34 + uBass * 0.10 + uBeat * 0.05 * seed;
      vec2 c = vec2(cos(ang), sin(ang)) * rad;
      float d = length(p - c);
      float size = 0.004 + bv * 0.012;
      float dot_ = 1.0 - smoothstep(size, size * 2.6, d);
      colour += mix(uAccent, uAccentStrong, bv) * dot_ * (0.35 + bv);
      alpha += dot_ * (0.35 + bv);
    }
    alpha = clamp(alpha, 0.0, 1.0);
  }

  if (alpha <= 0.001) discard;
  fragColour = vec4(colour, clamp(alpha, 0.0, 1.0));
}`;

function compile(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader | null {
  const shader = gl.createShader(type);
  if (!shader) return null;
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    // Surfaced rather than swallowed: a silent shader failure looks identical to
    // "there is no audio", which would send someone hunting the wrong problem.
    console.error('shader compile failed:', gl.getShaderInfoLog(shader));
    gl.deleteShader(shader);
    return null;
  }
  return shader;
}

/** Read a semantic colour token as 0..1 RGB for the shader. */
function tokenRgb(name: string, fallback: [number, number, number]): [number, number, number] {
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const parts = raw.split(/[\s,]+/).map(Number).filter((n) => Number.isFinite(n));
  if (parts.length < 3) return fallback;
  return [parts[0]! / 255, parts[1]! / 255, parts[2]! / 255];
}

interface Props {
  reader: AudioReader;
  mode: VisualiserMode;
  /** Frame cap. The overlay honours the user's performance preset. */
  fpsCap: number;
  height?: number;
}

export function Spectrum({ reader, mode, fpsCap, height = 120 }: Props) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const modeRef = useRef(mode);
  modeRef.current = mode;
  const fpsRef = useRef(fpsCap);
  fpsRef.current = fpsCap;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const gl = canvas.getContext('webgl2', {
      alpha: true,
      premultipliedAlpha: false,
      antialias: true,
      // The overlay composites over the desktop; a depth buffer is wasted memory.
      depth: false,
      stencil: false,
    });
    if (!gl) {
      setFailure('WebGL2 is unavailable, so the visualiser cannot run.');
      return;
    }

    const vs = compile(gl, gl.VERTEX_SHADER, VERTEX);
    const fs = compile(gl, gl.FRAGMENT_SHADER, FRAGMENT);
    if (!vs || !fs) {
      setFailure('The visualiser shader failed to compile.');
      return;
    }

    const program = gl.createProgram()!;
    gl.attachShader(program, vs);
    gl.attachShader(program, fs);
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      console.error('shader link failed:', gl.getProgramInfoLog(program));
      setFailure('The visualiser shader failed to link.');
      return;
    }
    gl.useProgram(program);

    const u = {
      data: gl.getUniformLocation(program, 'uData'),
      resolution: gl.getUniformLocation(program, 'uResolution'),
      time: gl.getUniformLocation(program, 'uTime'),
      mode: gl.getUniformLocation(program, 'uMode'),
      bass: gl.getUniformLocation(program, 'uBass'),
      beat: gl.getUniformLocation(program, 'uBeat'),
      accent: gl.getUniformLocation(program, 'uAccent'),
      accentStrong: gl.getUniformLocation(program, 'uAccentStrong'),
      bandCount: gl.getUniformLocation(program, 'uBandCount'),
      waveCount: gl.getUniformLocation(program, 'uWaveCount'),
    };

    // Two-row single-channel texture: bands on row 0, waveform on row 1.
    const texture = gl.createTexture();
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.uniform1i(u.data, 0);

    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

    const MODE_INDEX: Record<VisualiserMode, number> = {
      bars: 0,
      mirrored: 1,
      circular: 2,
      waveform: 3,
      particles: 4,
    };

    let raf = 0;
    let lastDraw = 0;
    let beatPulse = 0;
    let lastBeatAt = -1;
    let width = 0;
    let texWidth = 0;
    let pixels = new Uint8Array(0);
    const start = performance.now();

    const render = (now: number) => {
      raf = requestAnimationFrame(render);

      const minInterval = 1000 / Math.max(1, fpsRef.current);
      if (now - lastDraw < minInterval) return;
      lastDraw = now;

      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      const cssWidth = canvas.clientWidth;
      const cssHeight = canvas.clientHeight;
      const w = Math.max(1, Math.floor(cssWidth * dpr));
      const h = Math.max(1, Math.floor(cssHeight * dpr));
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
        gl.viewport(0, 0, w, h);
        width = w;
      }

      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);

      const features = reader();
      if (!features || features.bands.length === 0) return;

      const bandCount = features.bands.length;
      const waveCount = features.waveform.length;
      const cols = Math.max(bandCount, waveCount);
      if (cols !== texWidth) {
        texWidth = cols;
        pixels = new Uint8Array(cols * 2);
      }

      // Row 0: bands. Row 1: waveform, centred so zero is mid-grey.
      for (let i = 0; i < cols; i += 1) {
        const b = i < bandCount ? features.bands[i]! : 0;
        pixels[i] = Math.max(0, Math.min(255, Math.round(b * 255)));
        const s = i < waveCount ? features.waveform[i]! : 0;
        pixels[cols + i] = Math.max(0, Math.min(255, Math.round((s * 0.5 + 0.5) * 255)));
      }

      gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, cols, 2, 0, gl.RED, gl.UNSIGNED_BYTE, pixels);

      // Beat pulse decays over ~250 ms so a hit reads as a flash, not a step.
      if (features.beat && features.atMs !== lastBeatAt) {
        beatPulse = 1;
        lastBeatAt = features.atMs;
      } else {
        beatPulse = Math.max(0, beatPulse - minInterval / 250);
      }

      gl.uniform2f(u.resolution, canvas.width, canvas.height);
      gl.uniform1f(u.time, (now - start) / 1000);
      gl.uniform1i(u.mode, MODE_INDEX[modeRef.current]);
      gl.uniform1f(u.bass, features.bass);
      gl.uniform1f(u.beat, beatPulse);
      gl.uniform1f(u.bandCount, bandCount);
      gl.uniform1f(u.waveCount, Math.max(1, waveCount));
      gl.uniform3fv(u.accent, tokenRgb('--lq-accent', [0.42, 0.67, 1]));
      gl.uniform3fv(u.accentStrong, tokenRgb('--lq-accent-strong', [0.59, 0.78, 1]));

      gl.drawArrays(gl.TRIANGLES, 0, 3);
      void width;
    };

    raf = requestAnimationFrame(render);
    return () => {
      cancelAnimationFrame(raf);
      gl.deleteTexture(texture);
      gl.deleteProgram(program);
      gl.deleteShader(vs);
      gl.deleteShader(fs);
    };
  }, [reader]);

  if (failure) {
    return <p className="lq-muted lq-vis__failure">{failure}</p>;
  }

  return <canvas ref={canvasRef} className="lq-vis" style={{ height }} />;
}
