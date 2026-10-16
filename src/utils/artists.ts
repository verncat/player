/** Shown where a track has no artist tag. There's no page for it. */
export const UNKNOWN_ARTIST = 'Unknown artist';

/** The one spelling of "Various Artists": library.rs turns every alias in
 *  album artist tags into it. Must match VARIOUS_ARTISTS there. */
export const VARIOUS_ARTISTS = 'Various Artists';

/** Text to show for an artist, e.g. a track's `artist`. */
export function artistLabel(artist: string | null | undefined): string {
  return artist || UNKNOWN_ARTIST;
}

/** Whether `artist` has an artist page; unknown artists don't. */
export function hasArtistPage(artist: string | null | undefined): artist is string {
  return !!artist;
}

/** Various Artists doesn't perform tracks, so its page lists only albums. */
export function isVariousArtists(artist: string): boolean {
  return artist === VARIOUS_ARTISTS;
}
