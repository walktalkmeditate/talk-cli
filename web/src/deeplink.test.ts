import { describe, it, expect } from 'vitest';
import { parseHash, buildHash, shareUrl, isSafeId, neutralize } from './deeplink';

describe('parseHash', () => {
  it('parses a valid #q=<question-id> into a sanitized questionId', () => {
    // #given a hash carrying a real kebab question id
    // #when parsed
    const link = parseHash('#q=abc-123');
    // #then the id is returned verbatim
    expect(link).toEqual({ questionId: 'abc-123' });
  });

  it('ignores an absent q param (normal front-door selection)', () => {
    // #given a hash with no q
    // #then no questionId is produced
    expect(parseHash('')).toEqual({});
    expect(parseHash('#')).toEqual({});
    expect(parseHash('#other=value')).toEqual({});
  });

  it('ignores an uppercase / punctuated id (outside the safe alphabet)', () => {
    // #given ids that are not lowercase kebab
    // #then each is dropped, not accepted
    expect(parseHash('#q=AbC').questionId).toBeUndefined();
    expect(parseHash('#q=a.b').questionId).toBeUndefined();
    expect(parseHash('#q=a_b').questionId).toBeUndefined();
  });

  it('ignores an over-long id (length cap)', () => {
    // #given a 65-char id (one past the 64 cap)
    const hash = `#q=${'a'.repeat(65)}`;
    // #then it is rejected
    expect(parseHash(hash).questionId).toBeUndefined();
    // #and a 64-char id is accepted
    expect(parseHash(`#q=${'a'.repeat(64)}`).questionId).toBe('a'.repeat(64));
  });

  it('strips control/ESC bytes before the value can reach the terminal', () => {
    // #given a hash whose q carries a percent-encoded ESC + OSC introducer
    // (URLSearchParams decodes %1b%5d8 → "\x1b]8")
    const link = parseHash('#q=%1b%5d8abc');
    // #then the unsafe value is dropped entirely (control bytes neutralized, the
    //      residue then fails the alphabet test) — nothing reaches the terminal
    expect(link.questionId).toBeUndefined();
  });

  it('never leaves an ESC/control byte in any accepted id', () => {
    // #given a control byte spliced into an otherwise-kebab id
    const link = parseHash('#q=ab%1bcd');
    // #then either it is dropped, or (defensively) carries no control bytes
    if (link.questionId !== undefined) {
      // eslint-disable-next-line no-control-regex
      expect(/[\x00-\x1f\x7f-\x9f]/.test(link.questionId)).toBe(false);
    } else {
      expect(link.questionId).toBeUndefined();
    }
  });
});

describe('buildHash / shareUrl round-trip', () => {
  it('round-trips a valid id through buildHash → parseHash', () => {
    // #given a valid id
    const id = 'ready-to-let-go';
    // #then it survives the build/parse round-trip unchanged
    expect(parseHash(buildHash(id))).toEqual({ questionId: id });
  });

  it('builds an empty hash for an unsafe id (cannot smuggle it into a URL)', () => {
    expect(buildHash('Bad Id!')).toBe('');
    expect(buildHash('a'.repeat(65))).toBe('');
  });

  it('composes a full share URL from a base', () => {
    expect(shareUrl('held-runs', 'https://talk.pilgrimapp.org/')).toBe(
      'https://talk.pilgrimapp.org/#q=held-runs',
    );
  });

  it('yields the bare base (no fragment) for an unsafe id', () => {
    expect(shareUrl('NOPE', 'https://talk.pilgrimapp.org/')).toBe('https://talk.pilgrimapp.org/');
  });
});

describe('SAFE_ID alphabet (isSafeId)', () => {
  it('accepts lowercase kebab ids in 1..=64 chars', () => {
    expect(isSafeId('a')).toBe(true);
    expect(isSafeId('grateful-this-moment')).toBe(true);
    expect(isSafeId('abc-123')).toBe(true);
    expect(isSafeId('a'.repeat(64))).toBe(true);
  });

  it('rejects empty, uppercase, punctuation, spaces, and over-long ids', () => {
    expect(isSafeId('')).toBe(false);
    expect(isSafeId('UPPER')).toBe(false);
    expect(isSafeId('a b')).toBe(false);
    expect(isSafeId('a.b')).toBe(false);
    expect(isSafeId('a/b')).toBe(false);
    expect(isSafeId('a'.repeat(65))).toBe(false);
  });
});

describe('neutralize', () => {
  it('strips C0/C1 control and DEL bytes', () => {
    // #given a string laced with ESC, NUL, and a C1 byte
    const dirty = `a\x1bb\x00c\x9fd\x7fe`;
    // #when neutralized
    const clean = neutralize(dirty);
    // #then no control byte remains, the printable residue survives
    // eslint-disable-next-line no-control-regex
    expect(/[\x00-\x1f\x7f-\x9f]/.test(clean)).toBe(false);
    expect(clean).toBe('abcde');
  });

  it('caps length at 64', () => {
    expect(neutralize('a'.repeat(200)).length).toBe(64);
  });
});
