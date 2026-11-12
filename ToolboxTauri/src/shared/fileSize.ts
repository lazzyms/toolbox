const units = ["B", "KB", "MB", "GB", "TB"] as const;

export const formatBytes = (bytes: number): string => {
  let value = Math.max(0, bytes);
  let unit = 0;
  while (unit < units.length - 1 && Math.round(value * 10) / 10 >= 1000) {
    value /= 1000;
    unit += 1;
  }
  const shown = unit <= 1 ? Math.round(value) : Math.round(value * 10) / 10;
  return `${shown} ${units[unit]}`;
};

export const percentSaved = (originalBytes: number, outputBytes: number): number | null =>
  originalBytes > 0 ? Math.round((1 - outputBytes / originalBytes) * 100) : null;

// A real saving never shows as 0%, and a non-empty output never shows as 100%.
const shownSaving = (originalBytes: number, outputBytes: number): number | null => {
  const saved = percentSaved(originalBytes, outputBytes);
  if (saved === null || outputBytes >= originalBytes) return null;
  return Math.min(outputBytes > 0 ? 99 : 100, Math.max(1, saved));
};

export const formatSavings = (originalBytes: number, outputBytes: number): string | null => {
  if (percentSaved(originalBytes, outputBytes) === null) return null;
  const shown = shownSaving(originalBytes, outputBytes);
  return shown === null ? "No savings" : `Saved ${shown}%`;
};

/** One file's sizes, as in "14.6 MB → 3.8 MB, 74% smaller". */
export const formatSizeChange = (originalBytes: number, outputBytes: number): string => {
  const shown = shownSaving(originalBytes, outputBytes);
  return `${formatBytes(originalBytes)} → ${formatBytes(outputBytes)}, ${shown === null ? "no savings" : `${shown}% smaller`}`;
};

/** A batch's saving, as in "Saved 17.1 MB, 67% smaller". */
export const formatBatchSaving = (originalBytes: number, outputBytes: number): string => {
  const shown = shownSaving(originalBytes, outputBytes);
  return shown === null ? "No savings" : `Saved ${formatBytes(originalBytes - outputBytes)}, ${shown}% smaller`;
};
