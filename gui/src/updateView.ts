/**
 * What the update dialog runs, from what the user chose: kept apart from the
 * dialog so the rules can be tested.
 */
import type { Format, PlanView } from './types';

/** An update to run: an app, and with several installs, one of its formats. */
export type Unit = { id: string; plan: PlanView | null; error?: string };

/**
 * For an AppImage while FUSE is missing: install it anyway (it starts once
 * FUSE is installed), install it to unpack and run on each launch, or leave
 * it out.
 */
export type FuseChoice = 'anyway' | 'unpack' | 'skip';

export const unitKey = (id: string, format: Format) => `${id}/${format}`;

const keyOf = (u: Unit) => unitKey(u.id, u.plan!.format);

/** Updates the user has to choose for before anything runs: FUSE is missing for them and nothing is chosen yet. */
export function awaitingFuseChoice(units: Unit[], choices: Record<string, FuseChoice>): Unit[] {
  return units.filter((u) => u.plan?.asks_about_fuse && !choices[keyOf(u)]);
}

/** The planned updates that run: not held back (`held`) and not left out for FUSE. */
export function runnableUnits(units: Unit[], held: string[], choices: Record<string, FuseChoice>): Unit[] {
  return units.filter((u) => u.plan && !held.includes(keyOf(u)) && !(u.plan.asks_about_fuse && choices[keyOf(u)] === 'skip'));
}

/** Whether to install the update to unpack and run, as chosen for it. */
export function unpacks(unit: Unit, choices: Record<string, FuseChoice>): boolean {
  return Boolean(unit.plan?.asks_about_fuse) && choices[keyOf(unit)] === 'unpack';
}
