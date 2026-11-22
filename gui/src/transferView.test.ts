import { describe, expect, it } from 'vitest';
import { choicesComplete, choicesFor, describeTransfer, needsChoice } from './transferView';
import type { DataMove } from './types';

const move = (name: string, conflict: boolean, bytes = 2048): DataMove => ({
  kind: 'config',
  name,
  from: `/home/me/.config/${name}`,
  to: `/home/me/.var/app/org.example.Tool/config/${name}`,
  conflict,
  bytes,
});

describe('copying settings and data across a switch', () => {
  const moves = [move('tool', false), move('Tool', true), move('tl', true)];

  it('asks only about folders already at the new place', () => {
    expect(needsChoice(moves)).toEqual([1, 2]);
    expect(needsChoice([move('tool', false)])).toEqual([]);
  });

  it('waits for a choice for every conflict', () => {
    expect(choicesComplete(moves, {})).toBe(false);
    expect(choicesComplete(moves, { 1: 'use_old' })).toBe(false);
    expect(choicesComplete(moves, { 1: 'use_old', 2: 'keep_new' })).toBe(true);
    expect(choicesComplete(moves, { 1: 'replace', 2: 'keep_new' })).toBe(true);
    expect(choicesComplete(moves, { 1: 'copy', 2: 'keep_new' })).toBe(false);
    expect(choicesComplete([move('tool', false)], {})).toBe(true);
  });

  it('copies what has nothing in the way and never copies over a conflict unasked', () => {
    expect(choicesFor(moves, { 1: 'use_old' })).toEqual(['copy', 'use_old', 'keep_new']);
  });

  it('says what was copied', () => {
    const results = moves.map((m, i) => ({ ...m, copied: i < 2, error: null }));
    expect(describeTransfer(results)).toBe('copied 2 folders of settings and data (4 KiB)');
    expect(describeTransfer([{ ...moves[1], copied: false, error: null }])).toBe('settings and data left as they were');
    expect(describeTransfer([{ ...moves[0], copied: false, error: 'disk full' }])).toBe(
      'could not copy /home/me/.config/tool: disk full',
    );
  });
});
