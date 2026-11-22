// Copying an app's settings and data after it switches into or out of a
// Flatpak: which folders need the user's choice, and what to say after.
import { formatSize } from './assetsView';
import type { DataChoice, DataMove, TransferResult } from './types';

/** What the user picked for the folders already at the new place, by their index in the moves. */
export type Picked = Record<number, DataChoice>;

/** Indexes of the folders that need a choice: something is already at their new place. */
export function needsChoice(moves: DataMove[]): number[] {
  return moves.flatMap((m, i) => (m.conflict ? [i] : []));
}

/** Whether every folder already at its new place has a choice. */
export function choicesComplete(moves: DataMove[], picked: Picked): boolean {
  return needsChoice(moves).every((i) => picked[i] !== undefined && picked[i] !== 'copy');
}

/** One choice per move, for `transferData`: folders with nothing in the way are copied. */
export function choicesFor(moves: DataMove[], picked: Picked): DataChoice[] {
  return moves.map((m, i) => (m.conflict ? (picked[i] ?? 'keep_new') : 'copy'));
}

/** A line for the results, e.g. "copied 2 folders of settings and data (1.2 MiB)". */
export function describeTransfer(results: TransferResult[]): string {
  const copied = results.filter((r) => r.copied);
  const failed = results.filter((r) => r.error);
  const parts: string[] = [];
  if (copied.length > 0) {
    const size = formatSize(copied.reduce((sum, r) => sum + r.bytes, 0));
    parts.push(`copied ${copied.length} folder${copied.length > 1 ? 's' : ''} of settings and data${size ? ` (${size})` : ''}`);
  } else if (failed.length === 0) {
    parts.push('settings and data left as they were');
  }
  if (failed.length > 0) parts.push(`could not copy ${failed.map((r) => r.from).join(', ')}: ${failed[0].error}`);
  return parts.join('; ');
}
