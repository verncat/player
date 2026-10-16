import { invoke } from "@tauri-apps/api/core";
import type { AlbumSummary, Track, TrackPage } from "./types";

const MAX_PAGE_SIZE = 100;

// Flexible-playlist rules, as compiled by compile_smart_playlist_sql() in
// library.rs. A group combines its rules with its own match mode.
interface Rule { field: string; op: string; value: string }
interface RuleGroup { match: 'all' | 'any'; rules: RuleEntry[] }
type RuleEntry = Rule | RuleGroup;

const isIn = (field: string, value: string): Rule => ({ field, op: 'in', value: JSON.stringify([value]) });
const sortBy = (field: string): Rule => ({ field: 'sort', op: 'sort_asc', value: field });

// Runs rules (top level: all must match) and returns one page of results.
function fetchRulesPage(rules: RuleEntry[], includeDuplicates: boolean, limit: number, offset: number) {
  return invoke<TrackPage>('get_smart_playlist_tracks_page', {
    rulesJson: JSON.stringify(rules),
    matchMode: 'all',
    limit,
    offset,
    includeDuplicates,
  });
}

// Same, but loads every page.
async function fetchAllByRules(rules: RuleEntry[], includeDuplicates: boolean): Promise<Track[]> {
  const tracks: Track[] = [];
  for (;;) {
    const page = await fetchRulesPage(rules, includeDuplicates, MAX_PAGE_SIZE, tracks.length);
    tracks.push(...page.tracks);
    if (!page.has_more || page.tracks.length === 0) return tracks;
  }
}

// Albums are identified by name and album_artist_or_artist.
function albumRules(album: string, albumArtistOrArtist: string): Rule[] {
  return [
    isIn('album', album),
    // Empty when the album's tracks have no artist at all; then the name alone has to do.
    ...(albumArtistOrArtist ? [isIn('album_artist_or_artist', albumArtistOrArtist)] : []),
  ];
}

/** All tracks of an album, in disc and track-number order. */
export function fetchAlbumTracks(album: string, albumArtistOrArtist: string, includeDuplicates: boolean) {
  return fetchAllByRules([
    ...albumRules(album, albumArtistOrArtist),
    sortBy('disc_number'),
    sortBy('track_number'),
    sortBy('title'),
  ], includeDuplicates);
}

/** How many tracks an album has. */
export async function countAlbumTracks(album: string, albumArtistOrArtist: string, includeDuplicates: boolean) {
  const page = await fetchRulesPage(albumRules(album, albumArtistOrArtist), includeDuplicates, 1, 0);
  return page.total;
}

/** Tracks on an artist's page: theirs anywhere, plus every track on albums
 *  they're the album artist of. In disc and track-number order; ArtistView
 *  puts them into album order. */
export function fetchArtistTracks(artist: string, includeDuplicates: boolean) {
  return fetchAllByRules([
    { match: 'any', rules: [isIn('artist', artist), isIn('album_artist_or_artist', artist)] },
    sortBy('disc_number'),
    sortBy('track_number'),
    sortBy('title'),
  ], includeDuplicates);
}

/** Albums the artist has tracks on, oldest first. */
export function fetchArtistAlbums(artist: string, includeDuplicates: boolean) {
  return invoke<AlbumSummary[]>('get_artist_albums', { artist, includeDuplicates });
}
