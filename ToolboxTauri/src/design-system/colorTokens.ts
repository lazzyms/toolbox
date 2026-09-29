export const colorTokens = {
  documentText: "--color-document-text",
  documentHighlight: "--color-document-highlight",
} as const;

export type ColorTokenName = (typeof colorTokens)[keyof typeof colorTokens];

export function readColorToken(name: ColorTokenName): string {
  const value = getComputedStyle(document.body).getPropertyValue(name).trim();
  if (!value) throw new Error(`Design token ${name} is missing.`);
  return value;
}
