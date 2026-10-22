export type HexColor = `#${string}`;

type Rgb = [number, number, number];

function channels(hex: HexColor): Rgb {
  const digits = hex.slice(1);
  const full = digits.length === 3 ? [...digits].map((d) => d + d).join('') : digits;
  return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16) / 255) as Rgb;
}

const toLinear = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);

/** WCAG relative luminance, from 0 (black) to 1 (white). */
function relativeLuminance(hex: HexColor): number {
  const [r, g, b] = channels(hex).map(toLinear);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio, from 1 (same luminance) to 21 (black on white). */
function contrastRatio(a: HexColor, b: HexColor): number {
  const [lighter, darker] = [relativeLuminance(a), relativeLuminance(b)].sort((x, y) => y - x);
  return (lighter + 0.05) / (darker + 0.05);
}

/** Black or white, whichever has the higher WCAG contrast against `background`. */
export function readableTextOn(background: HexColor): '#000' | '#fff' {
  return contrastRatio(background, '#000') >= contrastRatio(background, '#fff') ? '#000' : '#fff';
}
