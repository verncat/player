import { readableTextOn, type HexColor } from './utils/color';

export type ThemeMode = 'dark';

export interface Theme {
  mode: ThemeMode;
  accent: HexColor;
}

export const DEFAULT_THEME: Theme = { mode: 'dark', accent: '#1db954' };

/**
 * Each color is set on the document root as the CSS variable `--color-<name>`,
 * so styles use e.g. `var(--color-text-muted)`.
 */
export interface ThemeColors {
  /** Window background. */
  bg: string;
  /** Main content area and drawers, one step above the background. */
  'bg-elevated': string;
  /** Cards and the player bar. */
  card: string;
  'card-hover': string;
  /** Inputs, menus and artwork placeholders. */
  surface: string;
  'surface-hover': string;
  /** Behind full-window overlays. */
  backdrop: string;
  /** Drop shadows and dimming layers, mixed with transparency. */
  shadow: string;
  text: string;
  /** Secondary text: metadata, labels. */
  'text-muted': string;
  /** Hints and disabled text. */
  'text-faint': string;
  border: string;
  /** Outlines of controls such as pill buttons and selects. */
  'border-strong': string;
  accent: string;
  'accent-hover': string;
  /** Text and icons drawn on an accent background. */
  'on-accent': string;
  /** Accent-colored text on the regular background. */
  'accent-text': string;
  danger: string;
  /** Error text on the regular background. */
  'danger-text': string;
  warning: string;
  info: string;
}

type AccentColor = 'accent' | 'on-accent';
type ModePalette = Omit<ThemeColors, AccentColor>;

// Accent shades are mixed from the chosen accent in CSS, so changing the
// accent only needs `--color-accent` and `--color-on-accent` to be updated.
const darkPalette: ModePalette = {
  bg: '#000',
  'bg-elevated': '#121212',
  card: '#181818',
  'card-hover': '#282828',
  surface: '#282828',
  'surface-hover': '#3e3e3e',
  backdrop: 'rgb(0 0 0 / 85%)',
  shadow: '#000',
  text: '#fff',
  'text-muted': '#a7a7a7',
  'text-faint': '#777',
  border: '#282828',
  'border-strong': '#535353',
  'accent-hover': 'color-mix(in srgb, var(--color-accent) 85%, white)',
  'accent-text': 'color-mix(in srgb, var(--color-accent) 55%, white)',
  danger: '#ff5252',
  'danger-text': 'color-mix(in srgb, var(--color-danger) 60%, white)',
  warning: '#ffa726',
  info: '#4fc3f7',
};

const palettes: Record<ThemeMode, ModePalette> = { dark: darkPalette };

export function themeColors(theme: Theme): ThemeColors {
  return {
    ...palettes[theme.mode],
    accent: theme.accent,
    'on-accent': readableTextOn(theme.accent),
  };
}

export function applyTheme(theme: Theme, root: HTMLElement = document.documentElement) {
  for (const [name, value] of Object.entries(themeColors(theme))) {
    root.style.setProperty(`--color-${name}`, value);
  }
}
