import type { HexColor } from './utils/color';
import type { ThemeMode } from './theme';

export interface ColorPreset {
  id: string;
  label: string;
  colors: Record<ThemeMode, HexColor>;
}

/**
 * Accents share one perceived lightness and colorfulness (OKLCH) per mode, so none stands out.
 * The light versions are lighter and calmer, so black text on accent buttons reads easily.
 */
export const ACCENT_PRESETS: readonly ColorPreset[] = [
  { id: 'green', label: 'Green', colors: { dark: '#1db954', light: '#64cb7b' } },
  { id: 'teal', label: 'Teal', colors: { dark: '#00b0a2', light: '#61c5b9' } },
  { id: 'blue', label: 'Blue', colors: { dark: '#4c99f8', light: '#79b4ff' } },
  { id: 'violet', label: 'Violet', colors: { dark: '#a17ff5', light: '#b79ffe' } },
  { id: 'pink', label: 'Pink', colors: { dark: '#e266a7', light: '#f18bbd' } },
  { id: 'coral', label: 'Coral', colors: { dark: '#ea6a64', light: '#f88f88' } },
  { id: 'orange', label: 'Orange', colors: { dark: '#e1791b', light: '#ef9a5d' } },
  { id: 'gray', label: 'Gray', colors: { dark: '#9399a2', light: '#acb1b9' } },
];

/** Glows for the top of the home page, given as the gradient's top color. */
export const GRADIENT_PRESETS: readonly ColorPreset[] = [
  { id: 'purple', label: 'Purple', colors: { dark: '#4a2d8a', light: '#e8e3ff' } },
  { id: 'blue', label: 'Blue', colors: { dark: '#1a437e', light: '#d6e9ff' } },
  { id: 'teal', label: 'Teal', colors: { dark: '#005252', light: '#c6f2f1' } },
  { id: 'rose', label: 'Rose', colors: { dark: '#722148', light: '#ffdce9' } },
  { id: 'amber', label: 'Amber', colors: { dark: '#683600', light: '#fee2c9' } },
  { id: 'slate', label: 'Slate', colors: { dark: '#3a4555', light: '#e3e8f0' } },
];

/** A preset id or a custom color, as stored in preferences. */
export type ColorChoice = string | HexColor;

export const isCustomColor = (value: string): value is HexColor => /^#[0-9a-f]{6}$/i.test(value);

export const isKnownChoice = (presets: readonly ColorPreset[], value: string) =>
  isCustomColor(value) || presets.some((preset) => preset.id === value);

/** The color of a choice in the given mode: custom colors are used as picked. */
export function resolveChoice(presets: readonly ColorPreset[], choice: ColorChoice, mode: ThemeMode): HexColor | null {
  if (isCustomColor(choice)) return choice;
  return presets.find((preset) => preset.id === choice)?.colors[mode] ?? null;
}
