export function formatBytes(bytes: number | null | undefined) {
  if (bytes == null) return "—";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  const precision = unitIndex === 0 || value >= 100 ? 0 : 1;
  return `${value.toFixed(precision)} ${units[unitIndex]}`;
}

export function formatTransferRate(bytesPerSecond: number | null | undefined) {
  if (bytesPerSecond == null) return "—";
  return `${formatBytes(bytesPerSecond)}/s`;
}

export function formatSampleRate(sampleRate: number | null | undefined) {
  if (!sampleRate) return "";
  const khz = sampleRate / 1000;
  return `${Number.isInteger(khz) ? khz.toFixed(0) : khz.toFixed(1)} kHz`;
}

export function formatDuration(secs: number | null) {
  if (!secs) return "--:--";
  const m = Math.floor(secs / 60);
  return `${m}:${String(Math.floor(secs % 60)).padStart(2, "0")}`;
}

export function formatTime(s: number) {
  const m = Math.floor(s / 60);
  return `${m}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
}
