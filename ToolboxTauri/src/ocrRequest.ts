import type { OcrMode, OcrRequest, OutputLocation } from "./contracts";

export function ocrRequest(paths: string[], outputLocation: OutputLocation, pageRange: string | null, mode: OcrMode, force = false): OcrRequest {
  return { paths, pageRange, mode, force, languages: ["eng"], outputLocation };
}
