export const colorTokens = {
  documentText: "--pdf-document-text",
  documentHighlight: "--pdf-document-highlight",
} as const;

export type ColorTokenName = (typeof colorTokens)[keyof typeof colorTokens];

// Document colors are written into the PDF and bound to <input type="color">, both of which only accept #RRGGBB.
export function documentColor(name: ColorTokenName, value: string): string {
  const color = value.trim().toLowerCase();
  if (!color) throw new Error(`PDF document color token ${name} is missing.`);
  if (!/^#[0-9a-f]{6}$/.test(color)) throw new Error(`PDF document color token ${name} must be #RRGGBB, got ${color}.`);
  return color;
}

export function readColorToken(name: ColorTokenName): string {
  return documentColor(name, getComputedStyle(document.body).getPropertyValue(name));
}
