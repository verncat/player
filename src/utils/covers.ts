import { convertFileSrc } from "@tauri-apps/api/core";

/** Must match covers::SCHEME in covers.rs. */
const COVER_SCHEME = 'cover';

/** Must match CoverSize and Variant in covers.rs. */
export type CoverSize = 'small' | 'medium' | 'full';

/** URL of a stored cover, from the cover hash a Track or AlbumSummary has. */
export function coverUrl(hash: string | null | undefined, size: CoverSize): string | null {
  return hash ? convertFileSrc(`${hash}-${size}`, COVER_SCHEME) : null;
}
