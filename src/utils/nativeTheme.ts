import { getCurrentWindow } from '@tauri-apps/api/window';
import type { ThemeMode } from '../theme';

/**
 * Matches the desktop title bar to the theme; `null` follows the system. Android's status and
 * navigation bars are system interface, so they keep following the system theme (Tauri doesn't
 * support `setTheme` there).
 */
export function syncNativeTheme(mode: ThemeMode | null) {
  try {
    void getCurrentWindow().setTheme(mode).catch(() => {});
  } catch {
    // Outside Tauri (plain `pnpm dev`) there is no native window to update.
  }
}
