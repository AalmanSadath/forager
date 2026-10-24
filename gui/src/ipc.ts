/**
 * Every call into the Rust side, in one place and typed, as in BifroSSH.
 * Grouped to mirror `src-tauri/src/commands/*.rs`.
 */
import { invoke } from '@tauri-apps/api/core';
import type {
  AboutInfo,
  AppView,
  Format,
  GuiSettings,
  TimerState,
  DownloadedView,
  HistoryEvent,
  PlanResult,
  ParsedSource,
  Preview,
  SettingsInput,
  SignatureForm,
  SourceSpec,
  StartRequest,
  SystemAppearance,
} from './types';

// ── activity ────────────────────────────────────────────────────
export const listHistory = (limit: number) => invoke<HistoryEvent[]>('list_history', { limit });

// ── apps ────────────────────────────────────────────────────────
export const listApps = () => invoke<AppView[]>('list_apps');

// ── check ───────────────────────────────────────────────────────
/** Results arrive as `app-status` events; resolves when all are done. */
export const checkApps = (ids?: string[]) => invoke<void>('check_apps', { ids: ids ?? null });

// ── edit ────────────────────────────────────────────────────────
export const parseSource = (url: string, forge: string | null, pattern: string | null) =>
  invoke<ParsedSource>('parse_source', { url, forge, pattern });
export const preview = (spec: SourceSpec, settings: SettingsInput, refresh: boolean) =>
  invoke<Preview>('preview', { spec, settings, refresh });
export const addApp = (spec: SourceSpec, id: string, settings: SettingsInput, signature: SignatureForm) =>
  invoke<AppView>('add_app', { spec, id, settings, signature });
export const saveApp = (id: string, settings: SettingsInput, signature: SignatureForm) =>
  invoke<AppView>('save_app', { id, settings, signature });
/** Stops tracking `id`, first uninstalling its installs in these formats. */
export const removeApp = (id: string, uninstall: Format[]) => invoke<void>('remove_app', { id, uninstall });

// ── update ──────────────────────────────────────────────────────
/** One plan per install that needs updating, each in its own format; empty when up to date. */
export const planUpdate = (id: string) => invoke<PlanResult[]>('plan_update', { id });
/** Progress arrives as `update-progress` events. */
export const startUpdate = (id: string, format: Format) => invoke<DownloadedView>('start_update', { id, format });
export const finishUpdate = (id: string, format: Format) => invoke<AppView>('finish_update', { id, format });
export const cancelUpdate = (id: string, format: Format) => invoke<void>('cancel_update', { id, format });

// ── settings ────────────────────────────────────────────────────
export const getGuiSettings = () => invoke<GuiSettings>('get_gui_settings');
export const setGuiSettings = (settings: GuiSettings) => invoke<void>('set_gui_settings', { settings });
export const getTimer = () => invoke<TimerState>('get_timer');
export const setTimer = (enabled: boolean, autoUpdate: boolean) =>
  invoke<TimerState>('set_timer', { enabled, autoUpdate });
export const getTokens = () => invoke<Record<string, string>>('get_tokens');
export const setTokens = (tokens: Record<string, string>) => invoke<void>('set_tokens', { tokens });
export const aboutInfo = () => invoke<AboutInfo>('about_info');

// ── system ──────────────────────────────────────────────────────
export const systemAppearance = () => invoke<SystemAppearance>('system_appearance');
export const startRequest = () => invoke<StartRequest | null>('start_request');
