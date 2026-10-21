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

const NO_ART_COLORS: [string, string] = ["#282828", "#181818"];

export function hashToColors(hash: string | null): [string, string] {
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
