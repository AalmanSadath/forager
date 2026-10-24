/**
 * Every call into the Rust side, in one place and typed, as in BifroSSH.
 * Grouped to mirror `src-tauri/src/commands/*.rs`.
 */
import { invoke } from '@tauri-apps/api/core';
import type { AppView, SystemAppearance } from './types';

// ── apps ────────────────────────────────────────────────────────
export const listApps = () => invoke<AppView[]>('list_apps');

// ── check ───────────────────────────────────────────────────────
/** Results arrive as `app-status` events; resolves when all are done. */
export const checkApps = (ids?: string[]) => invoke<void>('check_apps', { ids: ids ?? null });

// ── system ──────────────────────────────────────────────────────
export const systemAppearance = () => invoke<SystemAppearance>('system_appearance');
