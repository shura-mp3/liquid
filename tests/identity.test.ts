import { describe, expect, it } from 'vitest';
import {
  durationBucket,
  hasVersionMarker,
  identityKey,
  makeIdentity,
  normaliseArtist,
  normaliseTitle,
  offsetKey,
  sameTrack,
} from '../src/core/identity';

describe('normaliseTitle', () => {
  it('case-folds and collapses whitespace', () => {
    expect(normaliseTitle('  Hello   WORLD ')).toBe('hello world');
  });

  it('strips diacritics so sources that drop them still match', () => {
    expect(normaliseTitle('Café Déjà Vu')).toBe('cafe deja vu');
  });

  it('removes upload noise that describes the video, not the recording', () => {
    expect(normaliseTitle('Song Name (Official Music Video)')).toBe('song name');
    expect(normaliseTitle('Song Name [Official Audio]')).toBe('song name');
    expect(normaliseTitle('Song Name (Lyrics)')).toBe('song name');
    expect(normaliseTitle('Song Name (4K)')).toBe('song name');
  });

  it('KEEPS version markers — stripping them is how live takes get studio lyrics', () => {
    expect(normaliseTitle('Song Name (Live)')).toContain('live');
    expect(normaliseTitle('Song Name - Remastered')).toContain('remastered');
    expect(normaliseTitle('Song Name (Sped Up)')).toContain('sped up');
  });

  it('preserves non-Latin scripts', () => {
    expect(normaliseTitle('夜に駆ける')).toBe('夜に駆ける');
    expect(normaliseTitle('아이유 - 밤편지')).toBe('아이유 밤편지');
  });

  it('normalises punctuation differences between sources', () => {
    expect(normaliseTitle("Don't Stop Me Now!")).toBe(normaliseTitle('Dont Stop Me Now'));
  });
});

describe('normaliseArtist', () => {
  it('collapses to the primary artist across separator styles', () => {
    expect(normaliseArtist('Artist feat. Other')).toBe('artist');
    expect(normaliseArtist('Artist ft. Other')).toBe('artist');
    expect(normaliseArtist('Artist featuring Other')).toBe('artist');
    expect(normaliseArtist('Artist, Other')).toBe('artist');
    expect(normaliseArtist('Artist & Other')).toBe('artist');
    expect(normaliseArtist('Artist x Other')).toBe('artist');
  });

  it('returns an empty string for a missing artist rather than throwing', () => {
    expect(normaliseArtist(undefined)).toBe('');
    expect(normaliseArtist('')).toBe('');
  });

  it('does not split a name that merely contains a separator word', () => {
    expect(normaliseArtist('Andrew Bird')).toBe('andrew bird');
  });
});

describe('durationBucket', () => {
  it('quantises so reporting jitter does not fork identity', () => {
    expect(durationBucket(200_000)).toBe(durationBucket(201_000));
    expect(durationBucket(200_000)).toBe(durationBucket(198_000));
  });

  it('separates a sped-up version into a different bucket', () => {
    expect(durationBucket(200_000)).not.toBe(durationBucket(170_000));
  });

  it('gives unknown duration its own bucket, not bucket zero', () => {
    expect(durationBucket(undefined)).toBe(-1);
    expect(durationBucket(0)).toBe(-1);
    expect(durationBucket(Number.NaN)).toBe(-1);
    expect(durationBucket(1_000)).not.toBe(-1);
  });
});

describe('hasVersionMarker', () => {
  it('detects markers that mean a different recording', () => {
    for (const t of [
      'Song (Live)',
      'Song - Remastered 2011',
      'Song (Acoustic)',
      'Song (Remix)',
      'Song (sped up)',
      'Song (Nightcore)',
      'Song - Radio Edit',
    ]) {
      expect(hasVersionMarker(t), t).toBe(true);
    }
  });

  it('does not flag an ordinary title', () => {
    expect(hasVersionMarker('Bohemian Rhapsody')).toBe(false);
  });
});

describe('sameTrack', () => {
  const base = { provider: 'gsmtc' as const, title: 'Song', artist: 'Artist', durationMs: 200_000 };

  it('matches the same song reported slightly differently', () => {
    expect(
      sameTrack(
        makeIdentity(base),
        makeIdentity({ ...base, title: 'Song (Official Music Video)', durationMs: 201_500 }),
      ),
    ).toBe(true);
  });

  it('separates a live version from the studio version', () => {
    expect(sameTrack(makeIdentity(base), makeIdentity({ ...base, title: 'Song (Live)' }))).toBe(
      false,
    );
  });

  it('separates a sped-up version by duration', () => {
    expect(sameTrack(makeIdentity(base), makeIdentity({ ...base, durationMs: 160_000 }))).toBe(
      false,
    );
  });

  it('prefers a provider-native id when both sides have one', () => {
    const a = makeIdentity({ ...base, providerTrackId: 'abc' });
    const b = makeIdentity({ ...base, providerTrackId: 'abc', title: 'Totally Different' });
    expect(sameTrack(a, b)).toBe(true);
  });

  it('distinguishes different provider ids even with identical metadata', () => {
    const a = makeIdentity({ ...base, providerTrackId: 'abc' });
    const b = makeIdentity({ ...base, providerTrackId: 'xyz' });
    expect(sameTrack(a, b)).toBe(false);
  });
});

describe('keys', () => {
  it('identityKey is stable and version-prefixed', () => {
    const k = identityKey(makeIdentity({ provider: 'gsmtc', title: 'Song', artist: 'A', durationMs: 1_000 }));
    expect(k.startsWith('v1|')).toBe(true);
    expect(k).toBe(
      identityKey(makeIdentity({ provider: 'gsmtc', title: 'Song', artist: 'A', durationMs: 1_000 })),
    );
  });

  it('offsetKey ignores the provider so an offset carries across sources', () => {
    const fromSpotify = makeIdentity({ provider: 'gsmtc', title: 'Song', artist: 'A', durationMs: 200_000 });
    const fromExtension = makeIdentity({
      provider: 'extension-youtube',
      title: 'Song',
      artist: 'A',
      durationMs: 200_000,
    });
    expect(offsetKey(fromSpotify)).toBe(offsetKey(fromExtension));
    expect(identityKey(fromSpotify)).not.toBe(identityKey(fromExtension));
  });
});
