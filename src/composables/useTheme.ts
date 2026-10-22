import { computed, reactive, ref, watch } from 'vue';
import { ACCENT_PRESETS, GRADIENT_PRESETS, isKnownChoice } from '../colorPresets';
import { applyTheme, DEFAULT_THEME, type Theme, type ThemeMode } from '../theme';
import { syncNativeTheme } from '../utils/nativeTheme';
import { useAppearance } from './useAppearance';

export type ThemeModePreference = ThemeMode | 'system';

export interface ThemePreferences extends Omit<Theme, 'mode'> {
  mode: ThemeModePreference;
}

export const THEME_MODE_OPTIONS = [
  { value: 'system', label: 'System' },
  { value: 'dark', label: 'Dark' },
  { value: 'light', label: 'Light' },
] as const satisfies readonly { value: ThemeModePreference; label: string }[];

const STORAGE_KEY = 'player.theme';
const TRANSITION_MS = 350;

const preferences = reactive<ThemePreferences>({ ...DEFAULT_THEME });
const storageError = ref('');
const systemDarkQuery = window.matchMedia('(prefers-color-scheme: dark)');
const systemDark = ref(systemDarkQuery.matches);
systemDarkQuery.addEventListener('change', (event) => { systemDark.value = event.matches; });

try {
  const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}');
  if (THEME_MODE_OPTIONS.some((option) => option.value === saved?.mode)) preferences.mode = saved.mode;
  if (typeof saved?.accent === 'string' && isKnownChoice(ACCENT_PRESETS, saved.accent)) preferences.accent = saved.accent;
  if (saved?.gradient === null || (typeof saved?.gradient === 'string' && isKnownChoice(GRADIENT_PRESETS, saved.gradient))) preferences.gradient = saved.gradient;
} catch {
  // An unreadable preference never prevents the player from starting.
}

const theme = computed<Theme>(() => ({
  mode: preferences.mode === 'system' ? (systemDark.value ? 'dark' : 'light') : preferences.mode,
  accent: preferences.accent,
  gradient: preferences.gradient,
}));

function switchTheme(next: Theme) {
  const root = document.documentElement;
  if (useAppearance().reducedMotion.value) {
    applyTheme(next);
  } else if (document.startViewTransition) {
    document.startViewTransition(() => applyTheme(next));
  } else {
    root.classList.add('theme-transition');
    applyTheme(next);
    setTimeout(() => root.classList.remove('theme-transition'), TRANSITION_MS);
  }
}

/** Applies the saved theme without animation and keeps it applied as preferences change. */
export function startTheme() {
  document.documentElement.style.setProperty('--theme-transition-duration', `${TRANSITION_MS}ms`);
  applyTheme(theme.value);
  syncNativeTheme(preferences.mode === 'system' ? null : preferences.mode);
  watch(theme, switchTheme);
  watch(() => preferences.mode, (mode) => syncNativeTheme(mode === 'system' ? null : mode));
}

function setThemePreference<K extends keyof ThemePreferences>(key: K, value: ThemePreferences[K]) {
  const next = { ...preferences, [key]: value };
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
    Object.assign(preferences, next);
    storageError.value = '';
  } catch {
    storageError.value = 'Could not save the theme on this device. Please try again.';
  }
}

export function useTheme() {
  return { preferences, theme, storageError, setThemePreference };
}
