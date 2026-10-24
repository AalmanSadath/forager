import { create } from 'zustand';
import * as ipc from '../ipc';
import type { CheckEntry } from '../appsView';
import type { AppView, GuiSettings, StatusEvent, SystemAppearance } from '../types';
import { NO_APPEARANCE } from './theme';

/** Which page fills the window. An app's page is a full page, not a drawer. */
export type Page =
  | { kind: 'apps' }
  | { kind: 'app'; id: string }
  | { kind: 'add' }
  | { kind: 'activity' }
  | { kind: 'settings' };

export interface Toast {
  id: number;
  kind: 'info' | 'success' | 'error';
  text: string;
}

interface AppStore {
  page: Page;
  setPage: (page: Page) => void;

  apps: AppView[];
  appsError: string | null;
  loadApps: () => Promise<void>;

  /** Latest check per app id. */
  checks: Record<string, CheckEntry>;
  checkingAll: boolean;
  /** Checks the named apps, or all; results arrive through `applyStatus`. */
  checkApps: (ids?: string[]) => Promise<void>;
  applyStatus: (event: StatusEvent) => void;

  settings: GuiSettings;
  systemAppearance: SystemAppearance;
  setSystemAppearance: (a: SystemAppearance) => void;

  toasts: Toast[];
  toast: (kind: Toast['kind'], text: string) => void;
  dismissToast: (id: number) => void;
}

let nextToast = 1;

export const useAppStore = create<AppStore>()((set, get) => ({
  page: { kind: 'apps' },
  setPage: (page) => set({ page }),

  apps: [],
  appsError: null,
  loadApps: async () => {
    try {
      set({ apps: await ipc.listApps(), appsError: null });
    } catch (e) {
      set({ appsError: String(e) });
    }
  },

  checks: {},
  checkingAll: false,
  checkApps: async (ids) => {
    const targets = ids ?? get().apps.map((a) => a.id);
    set((s) => {
      const checks = { ...s.checks };
      for (const id of targets) {
        checks[id] = { status: checks[id]?.status ?? null, error: checks[id]?.error ?? null, checking: true };
      }
      return { checks, checkingAll: ids === undefined ? true : s.checkingAll };
    });
    try {
      await ipc.checkApps(ids);
    } catch (e) {
      get().toast('error', String(e));
    } finally {
      // Anything that never reported back is no longer being checked.
      set((s) => {
        const checks = { ...s.checks };
        for (const id of targets) if (checks[id]?.checking) checks[id] = { ...checks[id], checking: false };
        return { checks, checkingAll: ids === undefined ? false : s.checkingAll };
      });
    }
  },
  applyStatus: (event) =>
    set((s) => ({
      checks: { ...s.checks, [event.id]: { checking: false, status: event.status, error: event.error } },
    })),

  settings: { app_theme: 'system', accent_color: null },
  systemAppearance: NO_APPEARANCE,
  setSystemAppearance: (systemAppearance) => set({ systemAppearance }),

  toasts: [],
  toast: (kind, text) => {
    const id = nextToast++;
    set((s) => ({ toasts: [...s.toasts, { id, kind, text }] }));
    // Errors stay until dismissed; they may need reading twice.
    if (kind !== 'error') {
      setTimeout(() => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })), 5000);
    }
  },
  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));
