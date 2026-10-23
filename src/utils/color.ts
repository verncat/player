export type HexColor = `#${string}`;

type Rgb = [number, number, number];

function channels(hex: HexColor): Rgb {
  const digits = hex.slice(1);
  const full = digits.length === 3 ? [...digits].map((d) => d + d).join('') : digits;
  return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16) / 255) as Rgb;
}

function toHex(rgb: Rgb): HexColor {
  const byte = (c: number) => Math.round(Math.min(1, Math.max(0, c)) * 255).toString(16).padStart(2, '0');
  return `#${rgb.map(byte).join('')}`;
}

const toLinear = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const fromLinear = (c: number) => (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);

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

/** A color in OKLCH: perceptual lightness (0–1), chroma (colorfulness) and hue (degrees). */
export interface Oklch { l: number; c: number; h: number }

export function toOklch(hex: HexColor): Oklch {
  const [r, g, b] = channels(hex).map(toLinear);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const lightness = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const bAxis = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  return { l: lightness, c: Math.hypot(a, bAxis), h: ((Math.atan2(bAxis, a) * 180) / Math.PI + 360) % 360 };
}

function oklchToLinearRgb({ l: lightness, c, h }: Oklch): Rgb {
  const a = c * Math.cos((h * Math.PI) / 180);
  const b = c * Math.sin((h * Math.PI) / 180);
  const l = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (lightness - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

const inGamut = (rgb: Rgb) => rgb.every((c) => c >= -1e-4 && c <= 1 + 1e-4);

/** The OKLCH color as hex, lowering chroma until it fits the screen's gamut (keeping lightness and hue). */
export function fromOklch(color: Oklch): HexColor {
  let low = 0;
  let high = color.c;
  if (!inGamut(oklchToLinearRgb(color))) {
    for (let step = 0; step < 24; step++) {
      const c = (low + high) / 2;
      if (inGamut(oklchToLinearRgb({ ...color, c }))) low = c; else high = c;
    }
  } else {
    low = color.c;
  }
  return toHex(oklchToLinearRgb({ ...color, c: low }).map(fromLinear) as Rgb);
}
