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

export const formatSavings = (originalBytes: number, outputBytes: number): string | null => {
  const saved = percentSaved(originalBytes, outputBytes);
  if (saved === null) return null;
  if (outputBytes >= originalBytes) return "No savings";
  return `Saved ${Math.min(outputBytes > 0 ? 99 : 100, Math.max(1, saved))}%`;
};
