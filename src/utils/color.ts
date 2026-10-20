export type HexColor = `#${string}`;

function channels(hex: HexColor): [number, number, number] {
  const digits = hex.slice(1);
  const full = digits.length === 3 ? [...digits].map((d) => d + d).join('') : digits;
  return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16) / 255) as [number, number, number];
}

/** WCAG relative luminance, from 0 (black) to 1 (white). */
function relativeLuminance(hex: HexColor): number {
  const [r, g, b] = channels(hex).map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** Black or white, whichever has the higher WCAG contrast against `background`. */
export function readableTextOn(background: HexColor): '#000' | '#fff' {
  const luminance = relativeLuminance(background);
  const contrastWithBlack = (luminance + 0.05) / 0.05;
  const contrastWithWhite = 1.05 / (luminance + 0.05);
  return contrastWithBlack >= contrastWithWhite ? '#000' : '#fff';
}
