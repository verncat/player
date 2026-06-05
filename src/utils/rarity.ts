import { animatedRarities, rarityColors } from "../types";

export function rarityClass(r: string | null) {
  if (!r || r === "Common") return "";
  if (animatedRarities.has(r)) return `rarity-animated rarity-${r.toLowerCase()}`;
  return "rarity-tint";
}

export function rarityVars(r: string | null): Record<string, string> {
  if (!r || r === "Common") return {};
  const c = rarityColors[r] || "#b0b0b0";
  return { "--rc": c };
}

export function hashToColors(hash: string | null): [string, string] {
  if (!hash || hash.length < 8) return ["#1a1a2e", "#16213e"];
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
