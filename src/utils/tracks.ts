import type { Track } from "../types";

export function trackTagsList(track: Pick<Track, "tags">): string[] {
  if (!track.tags) return [];
  const seen = new Set<string>();
  const tags: string[] = [];
  for (const raw of track.tags.split(/[\n,;]+/)) {
    const tag = raw.trim();
    const key = tag.toLowerCase();
    if (!tag || seen.has(key)) continue;
    seen.add(key);
    tags.push(tag);
  }
  return tags;
}

export function trackTagsText(track: Pick<Track, "tags">) {
  return trackTagsList(track).join(", ");
}

export function normalizeTrackTagsInput(value: string): string | null {
  const tags = trackTagsList({ tags: value });
  return tags.length ? tags.join(", ") : null;
}

export function trackDateInputValue(value: number | null) {
  if (value == null) return "";
  const date = new Date(value * 1000);
  return Number.isNaN(date.getTime()) ? "" : date.toISOString().slice(0, 10);
}

export function parseTrackDateInput(value: string): number | null {
  if (!value) return null;
  const parsed = Date.parse(`${value}T00:00:00Z`);
  return Number.isNaN(parsed) ? null : Math.floor(parsed / 1000);
}

export function normalizeOptionalInteger(value: unknown): number | null {
  if (value == null || value === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? Math.trunc(parsed) : null;
}

export function normalizeNonNegativeInteger(value: unknown): number {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) return 0;
  return Math.max(0, Math.trunc(parsed));
}

export function normalizePath(path: string) {
  return path.replace(/\\/g, "/");
}
