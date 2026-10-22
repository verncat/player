import { animatedRarities, TRACK_RARITY_OPTIONS, type TrackRarity } from "../types";

/** The rarity a track is highlighted with: any known rarity except Common. */
export function highlightedRarity(r: string | null): TrackRarity | null {
  const rarity = TRACK_RARITY_OPTIONS.find((option) => option === r);
  return rarity && rarity !== "Common" ? rarity : null;
}

export function rarityColorName(rarity: TrackRarity) {
  return `rarity-${rarity.toLowerCase() as Lowercase<TrackRarity>}` as const;
}

export function rarityClass(r: string | null) {
  const rarity = highlightedRarity(r);
  if (!rarity) return "";
  if (animatedRarities.has(rarity)) return `rarity-animated rarity-${rarity.toLowerCase()}`;
  return "rarity-tint";
}

export function rarityVars(r: string | null): Record<string, string> {
  const rarity = highlightedRarity(r);
  return rarity ? { "--rc": `var(--color-${rarityColorName(rarity)})` } : {};
}
