import { describe, expect, it } from 'vitest';
import {
  AUTO_APPLY_THRESHOLD,
  pickAutoMatch,
  rankCandidates,
  scoreCandidate,
  similarity,
  type LyricsCandidate,
  type MatchQuery,
} from '../src/core/matcher';

const query: MatchQuery = {
  title: 'Bohemian Rhapsody',
  artist: 'Queen',
  album: 'A Night at the Opera',
  durationMs: 354_000,
};

function candidate(over: Partial<LyricsCandidate> = {}): LyricsCandidate {
  return {
    id: 1,
    trackName: 'Bohemian Rhapsody',
    artistName: 'Queen',
    albumName: 'A Night at the Opera',
    duration: 354,
    hasWordSync: true,
    syncedLyrics: '[00:00.00]x',
    ...over,
  };
}

describe('similarity', () => {
  it('is 1 for identical strings', () => {
    expect(similarity('hello', 'hello')).toBe(1);
  });

  it('is 0 for disjoint strings', () => {
    expect(similarity('abcdef', 'uvwxyz')).toBe(0);
  });

  it('is high for a small typo', () => {
    expect(similarity('bohemian rhapsody', 'bohemain rhapsody')).toBeGreaterThan(0.7);
  });

  it('handles single-character and empty input without dividing by zero', () => {
    expect(similarity('a', 'a')).toBe(1);
    expect(similarity('a', 'b')).toBe(0);
    expect(similarity('', '')).toBe(1);
    expect(Number.isFinite(similarity('', 'abc'))).toBe(true);
  });
});

describe('scoreCandidate — hard gates', () => {
  it('rejects a candidate whose duration is outside the +/-2s tolerance', () => {
    const r = scoreCandidate(query, candidate({ duration: 300 }));
    expect(r.verdict).toBe('reject');
    expect(r.score).toBe(0);
    expect(r.reasons.join()).toMatch(/duration differs/);
  });

  it('accepts a candidate inside the tolerance', () => {
    expect(scoreCandidate(query, candidate({ duration: 355 })).verdict).not.toBe('reject');
  });

  it('rejects the studio lyrics for a live recording', () => {
    const r = scoreCandidate(
      { ...query, title: 'Bohemian Rhapsody (Live Aid)' },
      candidate(),
    );
    expect(r.verdict).toBe('reject');
    expect(r.reasons.join()).toMatch(/alternate version/);
  });

  it('rejects a remix candidate for the original track', () => {
    const r = scoreCandidate(query, candidate({ trackName: 'Bohemian Rhapsody (Remix)' }));
    expect(r.verdict).toBe('reject');
  });

  it('allows a live track to match a live candidate', () => {
    const r = scoreCandidate(
      { ...query, title: 'Bohemian Rhapsody (Live)' },
      candidate({ trackName: 'Bohemian Rhapsody (Live)' }),
    );
    expect(r.verdict).not.toBe('reject');
  });

  it('notes when duration cannot be verified at all', () => {
    const r = scoreCandidate({ ...query, durationMs: undefined }, candidate());
    expect(r.reasons.join()).toMatch(/duration unknown/);
  });
});

describe('scoreCandidate — scoring', () => {
  it('auto-applies an exact match', () => {
    const r = scoreCandidate(query, candidate());
    expect(r.score).toBeGreaterThanOrEqual(AUTO_APPLY_THRESHOLD);
    expect(r.verdict).toBe('auto');
  });

  it('does not auto-apply a different song of the same length', () => {
    const r = scoreCandidate(query, candidate({ trackName: 'Killer Queen', albumName: 'Sheer Heart Attack' }));
    expect(r.verdict).not.toBe('auto');
  });

  it('penalises a candidate with no timing data', () => {
    const synced = scoreCandidate(query, candidate());
    const plain = scoreCandidate(
      query,
      candidate({ hasWordSync: false, syncedLyrics: null, plainLyrics: 'words' }),
    );
    expect(plain.score).toBeLessThan(synced.score);
    expect(plain.reasons.join()).toMatch(/no timing data/);
  });

  it('prefers word-level timings as a tie-breaker', () => {
    const word = scoreCandidate(query, candidate({ hasWordSync: true }));
    const line = scoreCandidate(query, candidate({ hasWordSync: false }));
    expect(word.score).toBeGreaterThan(line.score);
    expect(line.reasons.join()).toMatch(/line-level/);
  });

  it('treats an instrumental record as a real answer, not a failure', () => {
    const r = scoreCandidate(
      query,
      candidate({ instrumental: true, hasWordSync: false, syncedLyrics: null }),
    );
    expect(r.reasons.join()).toMatch(/instrumental/);
  });

  it('stays neutral when the album is unknown on one side', () => {
    const withAlbum = scoreCandidate(query, candidate());
    const withoutAlbum = scoreCandidate({ ...query, album: undefined }, candidate());
    // A missing album must not be treated as a mismatch.
    expect(Math.abs(withAlbum.score - withoutAlbum.score)).toBeLessThan(0.1);
    expect(withoutAlbum.verdict).toBe('auto');
  });

  it('keeps the score within 0..1', () => {
    const r = scoreCandidate(query, candidate());
    expect(r.score).toBeGreaterThanOrEqual(0);
    expect(r.score).toBeLessThanOrEqual(1);
  });

  it('explains a weak title match', () => {
    const r = scoreCandidate(query, candidate({ trackName: 'Completely Other Title' }));
    expect(r.reasons.join()).toMatch(/title similarity/);
  });
});

describe('rankCandidates', () => {
  it('orders best first and keeps rejections for explanation', () => {
    const ranked = rankCandidates(query, [
      candidate({ id: 'bad', trackName: 'Unrelated Song' }),
      candidate({ id: 'good' }),
      candidate({ id: 'wrong-length', duration: 120 }),
    ]);
    expect(ranked[0]!.candidate.id).toBe('good');
    expect(ranked).toHaveLength(3);
    expect(ranked.some((r) => r.verdict === 'reject')).toBe(true);
  });
});

describe('pickAutoMatch', () => {
  it('returns the clear winner', () => {
    const picked = pickAutoMatch(query, [candidate({ id: 'good' }), candidate({ id: 'bad', trackName: 'Other' })]);
    expect(picked?.candidate.id).toBe('good');
  });

  it('returns null when nothing clears the bar', () => {
    expect(pickAutoMatch(query, [candidate({ trackName: 'Unrelated Song' })])).toBeNull();
  });

  it('returns null for an empty candidate list', () => {
    expect(pickAutoMatch(query, [])).toBeNull();
  });

  it('refuses to guess between two indistinguishable top candidates', () => {
    const picked = pickAutoMatch(query, [
      candidate({ id: 'a' }),
      candidate({ id: 'b' }),
    ]);
    expect(picked).toBeNull();
  });
});
