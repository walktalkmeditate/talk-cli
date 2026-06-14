import { describe, it, expect } from 'vitest';
import { Repl } from './repl';

function type(repl: Repl, text: string): void {
  for (const ch of text) repl.handle(ch);
}

describe('Repl line editing', () => {
  it('inserts printable characters and submits on Enter', () => {
    const repl = new Repl();
    type(repl, 'box');
    expect(repl.input).toBe('box');
    const result = repl.handle('\r');
    expect(result.submitted).toBe('box');
    expect(repl.input).toBe('');
  });

  it('backspaces and ignores Enter on an empty line', () => {
    const repl = new Repl();
    type(repl, 'ab');
    repl.handle('\x7f');
    expect(repl.input).toBe('a');
    expect(repl.handle('\r').submitted).toBe('a');
    expect(repl.handle('\r').submitted).toBeUndefined();
  });

  it('Ctrl-U clears the line', () => {
    const repl = new Repl();
    type(repl, 'sound rain');
    repl.handle('\x15');
    expect(repl.input).toBe('');
  });

  it('strips control bytes from a paste and keeps the printable remainder', () => {
    // #given a paste with an embedded tab and a paste that starts with a control byte
    const repl = new Repl();
    // #when handled as single onData chunks (xterm delivers a paste as one chunk)
    repl.handle('a\tb');
    repl.handle('\x01cd');
    // #then only the printable characters land in the buffer
    expect(repl.input).toBe('abcd');
  });

  it('walks history with the arrow keys', () => {
    const repl = new Repl();
    type(repl, 'box');
    repl.handle('\r');
    type(repl, 'calm');
    repl.handle('\r');
    repl.handle('\x1b[A'); // up -> calm
    expect(repl.input).toBe('calm');
    repl.handle('\x1b[A'); // up -> box
    expect(repl.input).toBe('box');
    repl.handle('\x1b[B'); // down -> calm
    expect(repl.input).toBe('calm');
  });

  it('tab-completes a unique prefix and a common prefix', () => {
    const repl = new Repl(() => ['help', 'pause', 'pattern', 'man']);
    type(repl, 'he');
    repl.handle('\t');
    expect(repl.input).toBe('help '); // unique -> filled with trailing space

    const repl2 = new Repl(() => ['pattern', 'path']);
    type(repl2, 'pa');
    repl2.handle('\t');
    expect(repl2.input).toBe('pat'); // common prefix of pattern/path
  });

  it('renders the prompt with a cursor and a ghost hint when empty', () => {
    const repl = new Repl();
    const empty = repl.line('> ', 'type help');
    expect(empty).toContain('type help');
    expect(empty).toContain('\x1b[7m'); // inverse-video cursor cell
    type(repl, 'x');
    expect(repl.line('> ', 'type help')).not.toContain('type help');
  });
});
