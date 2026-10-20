import { inject, provide, type InjectionKey } from "vue";
import type { Track } from "../types";

export interface TrackActions {
  isCurrent(trackId: number): boolean;
  isNext(trackId: number): boolean;
  toggleLike(track: Track): void;
  identify(track: Track): void;
  edit(track: Track): void;
  openAddToPlaylist(track: Track, x: number, y: number): void;
  openMenu(track: Track, x: number, y: number, playlistId: number | null): void;
  openArtist(name: string): void;
  openAlbum(name: string, albumArtistOrArtist: string): void;
}

export type TrackButton = 'identify' | 'edit' | 'playCount' | 'like' | 'addToPlaylist';

const key: InjectionKey<TrackActions> = Symbol("TrackActions");

export function provideTrackActions(actions: TrackActions) {
  provide(key, actions);
}

export function useTrackActions(): TrackActions {
  const actions = inject(key);
  if (!actions) throw new Error("useTrackActions() needs provideTrackActions() in a parent component");
  return actions;
}
