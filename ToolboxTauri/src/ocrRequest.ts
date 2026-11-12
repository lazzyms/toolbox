import type { OcrRequest, OutputLocation } from "./contracts";

export function ocrRequest(paths: string[], outputLocation: OutputLocation, pageRange: string | null, language: string): OcrRequest {
  return { paths, pageRange, mode: "text", force: false, languages: [language], outputLocation };
}
