import { describe, it, expect, beforeAll } from 'vitest';
import { initWasmForTest } from './wasm-test-init';
import { Settle } from './wasm/talk_wasm.js';
import { Theme, parseComposed, type Rgb } from './theme';

/** Pull the truecolor RGB out of a rendered line's leading SGR escape. */
function toneRgb(rendered: string): Rgb | null {
  const m = rendered.match(/\x1b\[38;2;(\d+);(\d+);(\d+)m/);
  if (!m) return null;
  return [Number(m[1]), Number(m[2]), Number(m[3])];
}

/** WCAG sRGB relative luminance — to assert "settled brighter than edge". */
function relLum([r, g, b]: Rgb): number {
  const ch = (v: number): number => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b);
}

beforeAll(async () => {
  await initWasmForTest();
});

describe('compose → tone render path', () => {
  it('renders a fixed reflect screen with the expected (text, tone) sequence', () => {
    // #given a settle machine with one settled block and a live edge partial
    const settle = new Settle();
    settle.commit('um the raw words', 'The clean words.');
    settle.finalize();
    settle.onPartial('and a thought still forming');

    // #when composed and rendered through the rust theme
    const json = settle.compose(
      'reflect',
      'What am I avoiding?',
      '', // held_label
      true, // listening
      '2:14',
      'Light',
      false, // show_raw
      false, // paused
      false, // confirm_cancel
    );
    const lines = parseComposed(json);
    const theme = Theme.load('rust');

    // #then the question, settled text, and live edge each render in their tone.
    const question = lines.find((l) => l.text.includes('What am I avoiding?'));
    const settled = lines.find((l) => l.text.includes('The clean words.'));
    const edge = lines.find((l) => l.text.includes('and a thought still forming'));

    expect(question?.kind).toBe('question');
    expect(settled?.kind).toBe('settled');
    expect(edge?.kind).toBe('edge');

    const settledRgb = toneRgb(theme.render('settled', settled!.text));
    const edgeRgb = toneRgb(theme.render('edge', edge!.text));
    const questionRgb = toneRgb(theme.render('question', question!.text));

    // Settled = core; edge & question = dim — distinct tones.
    expect(settledRgb).toEqual([210, 146, 118]);
    expect(edgeRgb).toEqual([170, 124, 104]);
    expect(questionRgb).toEqual([170, 124, 104]);

    // Settled is brighter than the live edge / question.
    expect(relLum(settledRgb!)).toBeGreaterThan(relLum(edgeRgb!));
    expect(relLum(settledRgb!)).toBeGreaterThan(relLum(questionRgb!));

    settle.free();
  });

  it('joins composed lines into a CRLF frame body with each line painted', () => {
    const settle = new Settle();
    settle.commit('raw', 'A settled line.');
    settle.finalize();

    const json = settle.compose(
      'reflect',
      'Q?',
      '',
      false,
      '0:01',
      'Light',
      false,
      false,
      false,
    );
    const theme = Theme.load('rust');
    const body = theme.renderComposed(parseComposed(json));

    expect(body).toContain('\r\n'); // rows joined for one synchronized write
    expect(body).toContain('\x1b[38;2;210;146;118m'); // the settled line is core
    expect(body).toContain('A settled line.');

    settle.free();
  });
});
