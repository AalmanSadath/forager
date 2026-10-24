import { create } from 'zustand';
import * as ipc from '../ipc';
import type { CheckEntry } from '../appsView';
import type { AppView, GuiSettings, ProgressEvent, StatusEvent, SystemAppearance } from '../types';
import { NO_APPEARANCE } from './theme';

/** Which page fills the window. An app's page is a full page, not a drawer. */
export type Page =
  | { kind: 'apps' }
  | { kind: 'app'; id: string }
  | { kind: 'add'; url?: string }
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
  /** Checks apps whose last check is missing or older than half an hour. */
  checkStale: () => void;

  /** Apps the update dialog is working on; null when it is closed. */
  updating: string[] | null;
  openUpdate: (ids: string[]) => void;
  closeUpdate: () => void;
  /** 0..1 per app while it downloads or installs; null when unknown. */
  progress: Record<string, number | null>;
  applyProgress: (event: ProgressEvent) => void;
  clearProgress: (id: string) => void;

  settings: GuiSettings;
  loadSettings: () => Promise<void>;
  patchSettings: (patch: Partial<GuiSettings>) => void;
  systemAppearance: SystemAppearance;
  setSystemAppearance: (a: SystemAppearance) => void;

  toasts: Toast[];
  toast: (kind: Toast['kind'], text: string) => void;
  dismissToast: (id: number) => void;
}

let nextToast = 1;

/** Seconds after which a check is redone on start or when the window regains focus. */
const STALE_AFTER = 30 * 60;
const CHECKS_KEY = 'forager_checks';

/**
 * Successful checks survive a restart, so opening the window does not spend
 * a forge request per app each time; GitHub allows only 60 an hour without a
 * token. Errors are not kept: they should be retried.
 */
function loadSavedChecks(): Record<string, CheckEntry> {
  try {
    const saved = JSON.parse(localStorage.getItem(CHECKS_KEY) ?? '{}') as Record<string, CheckEntry>;
    for (const entry of Object.values(saved)) entry.checking = false;
    return saved;
  } catch {
    return {};
  }
}

function saveChecks(checks: Record<string, CheckEntry>) {
  const ok = Object.fromEntries(Object.entries(checks).filter(([, e]) => e.status && !e.error));
  try {
    localStorage.setItem(CHECKS_KEY, JSON.stringify(ok));
  } catch {
    // Storage full or unavailable: checks are only a cache.
  }
}

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

  checks: loadSavedChecks(),
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
    set((s) => {
      const checks = { ...s.checks, [event.id]: { checking: false, status: event.status, error: event.error } };
      saveChecks(checks);
      return { checks };
    }),
  checkStale: () => {
    const now = Date.now() / 1000;
    const stale = get()
      .apps.filter((a) => {
        const at = get().checks[a.id]?.status?.checked_at;
        return at === undefined || now - at > STALE_AFTER;
      })
      .map((a) => a.id);
    if (stale.length > 0) get().checkApps(stale);
  },

  updating: null,
  openUpdate: (ids) => set({ updating: ids }),
  closeUpdate: () => set({ updating: null }),
  progress: {},
  applyProgress: (e) => {
    const fraction =
      e.phase === 'download'
        ? e.total
          ? e.done / e.total
          : null
        : e.percent !== null
          ? e.percent / 100
          : null;
    set((s) => ({ progress: { ...s.progress, [e.id]: fraction } }));
  },
  clearProgress: (id) =>
    set((s) => {
      const progress = { ...s.progress };
      delete progress[id];
      return { progress };
    }),

  settings: { app_theme: 'system', accent_color: null },
  loadSettings: async () => {
    try {
      set({ settings: await ipc.getGuiSettings() });
    } catch {
      // Defaults stand in for a missing or unreadable file.
    }
  },
  patchSettings: (patch) => {
    const settings = { ...get().settings, ...patch };
    set({ settings });
    ipc.setGuiSettings(settings).catch((e) => get().toast('error', `Settings not saved: ${e}`));
  },
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
