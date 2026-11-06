import type { OcrRequest, OutputLocation } from "./contracts";

export function ocrRequest(paths: string[], outputLocation: OutputLocation, pageRange: string | null): OcrRequest {
  return { paths, pageRange, mode: "text", force: false, languages: ["eng"], outputLocation };
}
