export const colorTokens = {
  documentText: "--pdf-document-text",
  documentHighlight: "--pdf-document-highlight",
} as const;

export type ColorTokenName = (typeof colorTokens)[keyof typeof colorTokens];

export function readColorToken(name: ColorTokenName): string {
  const value = getComputedStyle(document.body).getPropertyValue(name).trim();
  if (!value) throw new Error(`PDF document color token ${name} is missing.`);
  return value;
}
