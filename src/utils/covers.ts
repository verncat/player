import { convertFileSrc } from "@tauri-apps/api/core";
import { activeThemeColors, activeThemeMode, type ThemeMode } from "../theme";

/** Must match covers::SCHEME in covers.rs. */
const COVER_SCHEME = 'cover';

/** Must match CoverSize and Variant in covers.rs. */
export type CoverSize = 'small' | 'medium' | 'full';

/** URL of a stored cover, from the cover hash a Track or AlbumSummary has. */
export function coverUrl(hash: string | null | undefined, size: CoverSize): string | null {
  return hash ? convertFileSrc(`${hash}-${size}`, COVER_SCHEME) : null;
}

// The hash picks the hues and a little variation; the mode picks the tone (HSL percentages,
// to which each track adds up to 31 saturation and 15 lightness): dark covers, light pastels.
const PLACEHOLDER_TONES: Record<ThemeMode, { saturation: number; lightness: [number, number] }> = {
  dark: { saturation: 45, lightness: [20, 15] },
  light: { saturation: 40, lightness: [74, 68] },
};

/** A two-color gradient standing in for a missing or loading cover, derived from the file hash. */
export function placeholderCoverColors(hash: string | null): [string, string] {
  if (!hash || hash.length < 8) return [activeThemeColors.value.surface, activeThemeColors.value.card];
  const a = parseInt(hash.slice(0, 6), 16);
  const b = parseInt(hash.slice(6, 12), 16);
  const hueA = a % 360;
  const hueB = (hueA + 137) % 360;
  const tone = PLACEHOLDER_TONES[activeThemeMode.value];
  const satA = tone.saturation + ((a >> 16) & 0x1f);
  const satB = tone.saturation + ((b >> 16) & 0x1f);
  const litA = tone.lightness[0] + ((a >> 8) & 0x0f);
  const litB = tone.lightness[1] + ((b >> 8) & 0x0f);
  return [
    `hsl(${hueA}, ${satA}%, ${litA}%)`,
    `hsl(${hueB}, ${satB}%, ${litB}%)`,
  ];
}
