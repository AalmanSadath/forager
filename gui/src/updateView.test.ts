import { describe, expect, it } from 'vitest';
import { awaitingFuseChoice, runnableUnits, unitKey, unpacks, type FuseChoice, type Unit } from './updateView';
import type { Format, PlanView } from './types';

const plan = (format: Format, asksAboutFuse = false): PlanView => ({
  from: null,
  to: 'v1.0',
  missing: false,
  format,
  lines: [],
  unverified: false,
  needs_root: false,
  downgrade: false,
  skip_after: null,
  notes: null,
  notes_releases: 0,
  asset_changed: null,
  fuse_missing: asksAboutFuse ? ['libfuse.so.2'] : [],
  fuse_packages: asksAboutFuse ? 'fuse fuse-libs' : null,
  asks_about_fuse: asksAboutFuse,
});

const appimage: Unit = { id: 'tool', plan: plan('appimage', true) };
const rpm: Unit = { id: 'other', plan: plan('rpm') };
const key = unitKey('tool', 'appimage');

describe('update choices', () => {
  it('waits for a choice on an AppImage while FUSE is missing', () => {
    expect(awaitingFuseChoice([appimage, rpm], {})).toEqual([appimage]);
    expect(awaitingFuseChoice([appimage, rpm], { [key]: 'anyway' })).toEqual([]);
    expect(awaitingFuseChoice([rpm], {})).toEqual([]);
  });

  it('leaves out what the user chose not to install, and what is held back', () => {
    const choices: Record<string, FuseChoice> = { [key]: 'skip' };
    expect(runnableUnits([appimage, rpm], [], choices)).toEqual([rpm]);
    expect(runnableUnits([appimage, rpm], [], { [key]: 'unpack' })).toEqual([appimage, rpm]);
    expect(runnableUnits([appimage, rpm], [unitKey('other', 'rpm')], {})).toEqual([appimage]);
    expect(runnableUnits([{ id: 'x', plan: null }], [], {})).toEqual([]);
  });

  it('unpacks only when chosen', () => {
    expect(unpacks(appimage, { [key]: 'unpack' })).toBe(true);
    expect(unpacks(appimage, { [key]: 'anyway' })).toBe(false);
    expect(unpacks(rpm, { [unitKey('other', 'rpm')]: 'unpack' })).toBe(false);
  });
});
