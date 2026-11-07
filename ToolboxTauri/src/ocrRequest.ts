import type { OcrMode, OcrRequest, OutputLocation } from "./contracts";
import { formatPageSpans, parsePageRanges, spansInPageOrder } from "./shared/pageRanges";

/** The range as it runs, in page order, or none when the field is empty. A range that
 * doesn't parse is sent as typed so the native boundary names the problem. */
const pagesInOrder = (pageRange: string | null) => {
  if (!pageRange?.trim()) return null;
  const parsed = parsePageRanges(pageRange);
  return parsed.ok ? formatPageSpans(spansInPageOrder(parsed.value)) : pageRange;
};

export function ocrRequest(paths: string[], outputLocation: OutputLocation, pageRange: string | null, mode: OcrMode, force = false, languages: string[] = ["eng"]): OcrRequest {
  return { paths, pageRange: pagesInOrder(pageRange), mode, force, languages, outputLocation };
}
