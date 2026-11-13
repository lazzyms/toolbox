import type { OcrMode, OcrRequest, OutputLocation } from "./contracts";

export function ocrRequest(paths: string[], outputLocation: OutputLocation, pageRange: string | null, language: string, mode: OcrMode): OcrRequest {
  return { paths, pageRange, mode, force: false, languages: [language], outputLocation };
}
