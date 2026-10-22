import { convertFileSrc } from "@tauri-apps/api/core";

/** Must match covers::SCHEME in covers.rs. */
const COVER_SCHEME = 'cover';

/** Must match CoverSize and Variant in covers.rs. */
export type CoverSize = 'small' | 'medium' | 'full';

/** URL of a stored cover, from the cover hash a Track or AlbumSummary has. */
export function coverUrl(hash: string | null | undefined, size: CoverSize): string | null {
  return hash ? convertFileSrc(`${hash}-${size}`, COVER_SCHEME) : null;
}

const NO_ART_COLORS: [string, string] = ["#282828", "#181818"];

/** A two-color gradient standing in for a missing or loading cover, derived from the file hash. */
export function placeholderCoverColors(hash: string | null): [string, string] {
  if (!hash || hash.length < 8) return NO_ART_COLORS;
  const a = parseInt(hash.slice(0, 6), 16);
  const b = parseInt(hash.slice(6, 12), 16);
  const hueA = a % 360;
  const hueB = (hueA + 137) % 360;
  const satA = 45 + ((a >> 16) & 0x1f);
  const satB = 45 + ((b >> 16) & 0x1f);
  const litA = 20 + ((a >> 8) & 0x0f);
  const litB = 15 + ((b >> 8) & 0x0f);
  return [
    `hsl(${hueA}, ${satA}%, ${litA}%)`,
    `hsl(${hueB}, ${satB}%, ${litB}%)`,
  ];
}
