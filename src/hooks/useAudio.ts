/**
 * Live audio features from the capture engine.
 *
 * Frames arrive ~50 times a second. They are deliberately **not** put into React
 * state: that would re-render every widget fifty times a second to move some bars.
 * Instead the latest frame is kept in a ref and read by the renderer inside its own
 * animation loop, the same arrangement the lyrics panel uses.
 *
 * Bands and waveform cross the IPC boundary quantised to bytes — finer than a
 * display pixel, and a fraction of the size of full floats at this rate.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';

import { ipc, type AudioFrame, type CaptureStatus } from '../providers/ipc';

/** Decoded, display-ready audio features. */
export interface AudioFeatures {
  /** 0..1 per band. */
  bands: Float32Array;
  /** -1..1. */
  waveform: Float32Array;
  rms: number;
  peak: number;
  bass: number;
  mid: number;
  treble: number;
  beat: boolean;
  silent: boolean;
  flux: number;
  /** Monotonic ms the frame was produced, on the native clock. */
  atMs: number;
}

export type AudioReader = () => AudioFeatures | null;

const EMPTY: AudioFeatures = {
  bands: new Float32Array(0),
  waveform: new Float32Array(0),
  rms: 0,
  peak: 0,
  bass: 0,
  mid: 0,
  treble: 0,
  beat: false,
  silent: true,
  flux: 0,
  atMs: 0,
};

function decode(frame: AudioFrame, into: AudioFeatures): AudioFeatures {
  const bands =
    into.bands.length === frame.bands.length ? into.bands : new Float32Array(frame.bands.length);
  for (let i = 0; i < frame.bands.length; i += 1) bands[i] = frame.bands[i]! / 255;

  const waveform =
    into.waveform.length === frame.waveform.length
      ? into.waveform
      : new Float32Array(frame.waveform.length);
  for (let i = 0; i < frame.waveform.length; i += 1) waveform[i] = frame.waveform[i]! / 127;

  return {
    bands,
    waveform,
    rms: frame.rms,
    peak: frame.peak,
    bass: frame.bass,
    mid: frame.mid,
    treble: frame.treble,
    beat: frame.beat,
    silent: frame.silent,
    flux: frame.flux,
    atMs: frame.atMs,
  };
}

export interface AudioView {
  /** Read the latest features without triggering a render. */
  reader: AudioReader;
  status: CaptureStatus | null;
  refreshStatus: () => void;
  /** True once at least one frame has arrived. */
  receiving: boolean;
}

export function useAudio(): AudioView {
  const latest = useRef<AudioFeatures>(EMPTY);
  const [status, setStatus] = useState<CaptureStatus | null>(null);
  const [receiving, setReceiving] = useState(false);
  const seen = useRef(false);

  useEffect(() => {
    const unlisten = listen<AudioFrame>('liquid://audio', (e) => {
      // Reuses the existing typed arrays where the shape is unchanged, so a
      // steady stream allocates nothing per frame.
      latest.current = decode(e.payload, latest.current);
      if (!seen.current) {
        seen.current = true;
        setReceiving(true);
      }
    });
    return () => {
      unlisten.then((un) => un()).catch(() => {});
    };
  }, []);

  const refreshStatus = useCallback(() => {
    ipc.getCaptureStatus().then(setStatus).catch(() => {});
  }, []);

  useEffect(() => {
    refreshStatus();
    // Capture health changes rarely; a slow poll is enough and costs nothing.
    const id = window.setInterval(refreshStatus, 2000);
    return () => window.clearInterval(id);
  }, [refreshStatus]);

  const reader = useCallback<AudioReader>(
    () => (seen.current ? latest.current : null),
    [],
  );

  return { reader, status, refreshStatus, receiving };
}
