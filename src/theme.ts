import { computed, shallowRef } from 'vue';
import { defaultTheme } from './themes/default';
import { ACCENT_PRESETS, GRADIENT_PRESETS, resolveChoice, type ColorChoice } from './colorPresets';
import { fromOklch, readableTextOn, toOklch, type HexColor } from './utils/color';

export type ThemeMode = 'dark' | 'light';

/**
 * Each color is set on the document root as the CSS variable `--color-<name>`,
 * so styles use e.g. `var(--color-text-muted)`.
 */
export interface ThemeColors {
  /** Window background and sidebar. */
  bg: string;
  /** Main content area and drawers. */
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
  /** Top of the home page header gradient (the content color when there is none). */
  hero: string;
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
  // Rarity colors must be plain colors: the 3D cover reads them as values, not CSS.
  'rarity-common': HexColor;
  'rarity-uncommon': HexColor;
  'rarity-rare': HexColor;
  'rarity-epic': HexColor;
  'rarity-legendary': HexColor;
  'rarity-mythic': HexColor;
}

type AccentColor = 'accent' | 'on-accent' | 'hero';
type NeutralColor = 'bg' | 'bg-elevated' | 'card' | 'card-hover' | 'surface' | 'surface-hover'
  | 'text' | 'text-muted' | 'text-faint' | 'border' | 'border-strong';
export type ModePalette = Omit<ThemeColors, AccentColor | NeutralColor> & Record<NeutralColor, HexColor>;

// The content area stays neutral: it already carries the header gradient.
const TINTED_COLORS: readonly Exclude<NeutralColor, 'bg-elevated'>[] = [
  'bg', 'card', 'card-hover', 'surface', 'surface-hover',
  'text', 'text-muted', 'text-faint', 'border', 'border-strong',
];

export interface ThemeDefinition {
  palettes: Record<ThemeMode, ModePalette>;
  accent?: Record<ThemeMode, HexColor>;
  gradient?: Record<ThemeMode, HexColor> | null;
  tint?: number;
}

export const activeDefinition = defaultTheme;

export interface Theme {
  mode: ThemeMode;
  accent: ColorChoice;
  gradient: ColorChoice | null;
  tint: number;
}

export const DEFAULT_THEME: Theme = {
  mode: 'dark',
  accent: ACCENT_PRESETS[0].id,
  gradient: GRADIENT_PRESETS[0].id,
  tint: 5,
};

const FULL_TINT_CHROMA = 0.05;

function tintGrays(palette: ModePalette, hue: number, chroma: number) {
  for (const name of TINTED_COLORS) palette[name] = fromOklch({ ...toOklch(palette[name]), c: chroma, h: hue });
}

/** Which color choices the active theme leaves to the user (settings shows only these). */
export const userColorChoices = {
  accent: activeDefinition.accent === undefined,
  gradient: activeDefinition.gradient === undefined,
  tint: activeDefinition.tint === undefined,
};

function accentColor(theme: Theme): HexColor {
  return activeDefinition.accent?.[theme.mode]
    ?? resolveChoice(ACCENT_PRESETS, theme.accent, theme.mode)
    ?? ACCENT_PRESETS[0].colors[theme.mode];
}

function heroColor(theme: Theme): HexColor | null {
  if (activeDefinition.gradient !== undefined) return activeDefinition.gradient?.[theme.mode] ?? null;
  if (theme.gradient === null) return null;
  return resolveChoice(GRADIENT_PRESETS, theme.gradient, theme.mode);
}

export function themeColors(theme: Theme): ThemeColors {
  const palette = { ...activeDefinition.palettes[theme.mode] };
  const accent = accentColor(theme);
  const { c: accentChroma, h: accentHue } = toOklch(accent);
  const tintChroma = Math.min(((activeDefinition.tint ?? theme.tint) / 100) * FULL_TINT_CHROMA, accentChroma);
  tintGrays(palette, accentHue, tintChroma);
  const hero = heroColor(theme);
  return {
    ...palette,
    accent,
    'on-accent': readableTextOn(accent),
    hero: hero ?? palette['bg-elevated'],
  };
}

const activeTheme = shallowRef(DEFAULT_THEME);

/** The applied theme's colors, for code that needs values instead of CSS variables. */
export const activeThemeColors = computed(() => themeColors(activeTheme.value));

export const activeThemeMode = computed(() => activeTheme.value.mode);

export function applyTheme(theme: Theme) {
  activeTheme.value = theme;
  const root = document.documentElement;
  root.style.colorScheme = theme.mode;
  for (const [name, value] of Object.entries(themeColors(theme))) {
    root.style.setProperty(`--color-${name}`, value);
  }
}

/** A theme color at the given opacity, for styles built in TypeScript. */
export function translucent(name: keyof ThemeColors, percent: number) {
  return `color-mix(in srgb, var(--color-${name}) ${percent}%, transparent)`;
}
