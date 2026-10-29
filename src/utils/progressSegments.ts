/** Equal-sized work items; failed indices are zero-based and processing reserves the next item. */
export function progressSegments(total: number, completed: number, failed: readonly number[], processing: boolean) {
  const count = Number.isFinite(total) ? Math.max(0, Math.floor(total)) : 0;
  const processed = Number.isFinite(completed) ? Math.max(0, Math.min(count, Math.floor(completed))) : 0;
  const unit = count ? 100 / count : 0;
  // Keep processing visible in large batches, fitting the marker inside the bar at its end.
  // Completed and failed portions always retain their actual share.
  const activeWidth = Math.max(unit, 5);
  const failures = [...new Set(failed.filter(index => Number.isInteger(index) && index >= 0 && index < processed))].sort((a, b) => a - b);
  const ranges: Array<{ start: number; count: number }> = [];
  for (const index of failures) {
    const last = ranges[ranges.length - 1];
    if (last && last.start + last.count === index) last.count++;
    else ranges.push({ start: index, count: 1 });
  }
  return {
    total: count,
    completed: processed,
    completedPercent: count ? processed / count * 100 : 0,
    failedCount: failures.length,
    errors: ranges.map(range => ({ ...range, left: range.start * unit, width: range.count * unit })),
    active: processed < count && processing
      ? { left: Math.min(processed * unit, 100 - activeWidth), width: activeWidth }
      : null,
  };
}
