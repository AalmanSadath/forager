/**
 * Every call into the Rust side, in one place and typed, as in BifroSSH.
 * Grouped to mirror `src-tauri/src/commands/*.rs`.
 */
import { invoke } from '@tauri-apps/api/core';
import type {
  AppView,
  ParsedSource,
  Preview,
  SettingsInput,
  SignatureForm,
  SourceSpec,
  StartRequest,
  SystemAppearance,
} from './types';

// ── apps ────────────────────────────────────────────────────────
export const listApps = () => invoke<AppView[]>('list_apps');

// ── check ───────────────────────────────────────────────────────
/** Results arrive as `app-status` events; resolves when all are done. */
export const checkApps = (ids?: string[]) => invoke<void>('check_apps', { ids: ids ?? null });

// ── edit ────────────────────────────────────────────────────────
export const parseSource = (url: string, forge: string | null) => invoke<ParsedSource>('parse_source', { url, forge });
export const preview = (spec: SourceSpec, settings: SettingsInput, refresh: boolean) =>
  invoke<Preview>('preview', { spec, settings, refresh });
export const addApp = (spec: SourceSpec, id: string, settings: SettingsInput, signature: SignatureForm) =>
  invoke<AppView>('add_app', { spec, id, settings, signature });
export const saveApp = (id: string, settings: SettingsInput, signature: SignatureForm) =>
  invoke<AppView>('save_app', { id, settings, signature });
export const removeApp = (id: string, uninstall: boolean) => invoke<void>('remove_app', { id, uninstall });

// ── system ──────────────────────────────────────────────────────
export const systemAppearance = () => invoke<SystemAppearance>('system_appearance');
export const startRequest = () => invoke<StartRequest | null>('start_request');
