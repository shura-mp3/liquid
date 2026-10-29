/**
 * Track information, artwork, progress and transport.
 *
 * Every piece of this reflects what the source actually reported. Absent metadata is
 * rendered as absent — no "Unknown Artist" filler — and a transport button is
 * disabled with a reason when the source did not advertise that capability, which
 * is common: browsers routinely expose next/previous but not seek.
 */

import { formatTime } from '../hooks/usePlayback';
import type { PlaybackSnapshot, TransportAction } from '../providers/ipc';
import type { ClockReading } from '../core/types';

interface Props {
  snapshot: PlaybackSnapshot | null;
  reading: ClockReading | null;
  artworkUrl: string | null;
  sourceName: string | null;
  onTransport: (action: TransportAction) => void;
  onSeek: (positionMs: number) => void;
  bridgeError: string | null;
}

export function NowPlaying({
  snapshot,
  reading,
  artworkUrl,
  sourceName,
  onTransport,
  onSeek,
  bridgeError,
}: Props) {
  if (bridgeError) {
    return (
      <div className="lq-np lq-np--empty">
        <p className="lq-np__empty-title">Media sessions unavailable</p>
        <p className="lq-np__empty-note">{bridgeError}</p>
      </div>
    );
  }

  if (!snapshot) {
    return (
      <div className="lq-np lq-np--empty">
        <p className="lq-np__empty-title">Nothing playing</p>
        <p className="lq-np__empty-note">
          Start a track in Spotify, a browser, or any player that reports to Windows.
        </p>
      </div>
    );
  }

  const position = reading?.positionMs ?? snapshot.positionMs;
  const duration = snapshot.durationMs;
  const progress = duration && duration > 0 ? Math.min(1, position / duration) : null;
  const { controls } = snapshot;
  const paused = snapshot.status !== 'playing';

  const seekFromEvent = (event: React.MouseEvent<HTMLDivElement>) => {
    if (!controls.seek || !duration) return;
    const rect = event.currentTarget.getBoundingClientRect();
    const ratio = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
    onSeek(Math.round(ratio * duration));
  };

  return (
    <div className="lq-np">
      <div className="lq-np__top">
        <div className="lq-np__art" data-empty={!artworkUrl}>
          {artworkUrl ? (
            <img src={artworkUrl} alt="" draggable={false} />
          ) : (
            <span className="lq-np__art-fallback" aria-hidden="true" />
          )}
        </div>

        <div className="lq-np__meta">
          <p className="lq-np__title" title={snapshot.title}>
            {snapshot.title}
          </p>
          {/* Rendered only when the source actually gave us one. */}
          {snapshot.artist && (
            <p className="lq-np__artist" title={snapshot.artist}>
              {snapshot.artist}
            </p>
          )}
          {snapshot.album && <p className="lq-np__album">{snapshot.album}</p>}
          {sourceName && (
            <p className="lq-np__source">
              <span className="lq-np__source-dot" data-playing={!paused} />
              {sourceName}
            </p>
          )}
        </div>
      </div>

      <div
        className="lq-np__progress lq-no-drag"
        data-seekable={controls.seek && !!duration}
        onClick={seekFromEvent}
        title={
          controls.seek
            ? 'Click to seek'
            : 'This source did not report that it supports seeking'
        }
      >
        <div
          className="lq-np__progress-fill"
          style={{ width: progress === null ? '0%' : `${progress * 100}%` }}
        />
      </div>

      <div className="lq-np__times">
        <span>{formatTime(position)}</span>
        {/* A source that never reported a duration gets --:-- rather than a guess. */}
        <span>{formatTime(duration)}</span>
      </div>

      <div className="lq-np__controls lq-no-drag">
        <TransportButton
          label="Previous"
          enabled={controls.previous}
          onClick={() => onTransport('previous')}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M7 6v12M19 6l-9 6 9 6z" />
          </svg>
        </TransportButton>

        <TransportButton
          label={paused ? 'Play' : 'Pause'}
          enabled={controls.play || controls.pause}
          primary
          onClick={() => onTransport('play-pause')}
        >
          {paused ? (
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M8 5l11 7-11 7z" />
            </svg>
          ) : (
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M8 5v14M16 5v14" />
            </svg>
          )}
        </TransportButton>

        <TransportButton
          label="Next"
          enabled={controls.next}
          onClick={() => onTransport('next')}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M17 6v12M5 6l9 6-9 6z" />
          </svg>
        </TransportButton>
      </div>
    </div>
  );
}

function TransportButton({
  label,
  enabled,
  primary,
  onClick,
  children,
}: {
  label: string;
  enabled: boolean;
  primary?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      className="lq-np__btn"
      data-primary={!!primary}
      disabled={!enabled}
      onClick={onClick}
      aria-label={label}
      // The tooltip is the explanation: a dead-looking button with no reason is
      // exactly what this project's rules forbid.
      title={enabled ? label : `${label} — this source did not report support for it`}
    >
      {children}
    </button>
  );
}
