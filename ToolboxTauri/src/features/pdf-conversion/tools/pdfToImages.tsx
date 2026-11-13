import { invoke } from "@tauri-apps/api/core";
import { exampleOutputName } from "../../../components/RunDestination";
import type { ToolResult } from "../../../contracts";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { capabilityFor } from "../../../registry/capabilities";
import { describeSpans, isShortened, pagesInSpans, parsePageRanges } from "../pageRanges";
import type { BatchPagesProps, ConversionTool } from "../tool";

type Resolution = "72" | "150" | "300" | "custom";
type PagesMode = "all" | "range";
type Options = { resolution: Resolution; customDpi: string; format: string; pages: PagesMode };

const MIN_DPI = 72;
const MAX_DPI = 300;
const PRESET_RESOLUTIONS = ["72", "150", "300"] as const;

const formatDetails: Record<string, { label: string; hint: string }> = {
  jpg: { label: "JPEG", hint: "Lossy at quality 90. Small files for photos and scans." },
  png: { label: "PNG", hint: "Lossless. Best for text and line art." },
  webp: { label: "WebP", hint: "Lossy at quality 90. Small files for the web." },
};

const imageFormats = () => (capabilityFor("pdf_to_images").outputFormats ?? []).map((extension) => extension.slice(1));

const formatLabel = (format: string) => formatDetails[format]?.label ?? format.toUpperCase();

const renderDpi = ({ resolution, customDpi }: Options) => {
  if (resolution !== "custom") return Number(resolution);
  const dpi = Number(customDpi);
  return customDpi.trim() && Number.isInteger(dpi) && dpi >= MIN_DPI && dpi <= MAX_DPI ? dpi : null;
};

const letterPixels = (dpi: number) => `${Math.ceil(8.5 * dpi)} × ${Math.ceil(11 * dpi)} px`;

const resolutionHint = (options: Options) => {
  const dpi = renderDpi(options);
  if (options.resolution === "custom") {
    return dpi === null
      ? `Enter a whole number from ${MIN_DPI} to ${MAX_DPI}.`
      : `Any value from ${MIN_DPI} to ${MAX_DPI}. At ${dpi} dpi a Letter page is ${letterPixels(dpi)}.`;
  }
  return `At ${dpi} dpi a Letter page is ${letterPixels(dpi ?? 0)}. Use 300 for print.`;
};

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

const plural = (count: number, one: string, many = `${one}s`) => `${count} ${count === 1 ? one : many}`;

const rangeSpans = (options: Options, pageRange: string) => {
  if (options.pages === "all") return { spans: null };
  const parsed = parsePageRanges(pageRange);
  return "error" in parsed ? { error: parsed.error } : { spans: parsed.spans };
};

const pagesFor = (options: Options, pageRange: string, pageCount: number) => {
  const range = rangeSpans(options, pageRange);
  if ("error" in range) return null;
  return range.spans ? pagesInSpans(range.spans, pageCount) : Array.from({ length: pageCount }, (_, index) => index);
};

const batchEstimate = ({ options, files, pageRange, picks, pageCounts }: BatchPagesProps<Options>) => {
  const estimate = { images: 0, made: 0, skipped: 0, shortened: 0, locked: 0, unreadable: 0, pending: 0 };
  const range = rangeSpans(options, pageRange);
  for (const path of files) {
    const count = pageCounts[path] ?? "pending";
    const picked = picks[path];
    const images = picked ? picked.length : typeof count === "number" ? pagesFor(options, pageRange, count)?.length ?? 0 : null;
    if (images === null) {
      estimate[count === "locked" ? "locked" : count === "unreadable" ? "unreadable" : "pending"] += 1;
      continue;
    }
    estimate.images += images;
    if (images > 0) estimate.made += 1;
    else estimate.skipped += 1;
    if (!picked && images > 0 && typeof count === "number" && range.spans && isShortened(range.spans, count)) estimate.shortened += 1;
  }
  return estimate;
};

const RunSummary = (props: BatchPagesProps<Options>) => {
  const range = rangeSpans(props.options, props.pageRange);
  if ("error" in range) {
    return (
      <div className="grid gap-0.5" role="status" aria-label="Image estimate">
        <p className="text-sm font-medium text-destructive">No image count yet</p>
        <p className="workspace-note">{props.pageRange.trim() ? "Fix the page range to convert" : "Enter a page range to convert"}</p>
      </div>
    );
  }
  const estimate = batchEstimate(props);
  const details = [
    `from ${plural(estimate.made, "PDF")}`,
    estimate.skipped && `${estimate.skipped} skipped`,
    estimate.locked && `${estimate.locked} locked`,
    estimate.unreadable && `${estimate.unreadable} unreadable`,
  ].filter(Boolean).join(", ");
  return (
    <div className="grid gap-0.5" role="status" aria-label="Image estimate">
      <p className="text-sm font-medium">{estimate.pending ? "Counting pages" : `This will make ${plural(estimate.images, "image")}`}</p>
      <p className="workspace-note">{estimate.pending ? `${plural(estimate.pending, "PDF")} left to count` : details}</p>
    </div>
  );
};

const rangeStatus = (props: BatchPagesProps<Options>, describedRange: string) => {
  const estimate = batchEstimate(props);
  const notes = [estimate.shortened && `${estimate.shortened} shortened`, estimate.skipped && `${estimate.skipped} will be skipped`].filter(Boolean);
  return `Range ${describedRange} applied to each PDF.${notes.length ? ` ${notes.join(", ")}.` : ""}`;
};

export const pdfToImages: ConversionTool<Options> = {
  initialOptions: { resolution: "150", customDpi: "200", format: "png", pages: "all" },
  inspectsPdf: true,
  outputPreview: (options) => {
    const dpi = renderDpi(options);
    return `Output preview unavailable. Export renders each PDF's pages at ${dpi ?? options.customDpi} dpi as ${formatLabel(options.format)} image files.`;
  },
  exportBlocked: (options, _files, pageRange) => renderDpi(options) === null || "error" in rangeSpans(options, pageRange),
  pagesPerPdf: pagesFor,
  RunSummary,
  OptionsPanel: ({ options, setOptions, files, pageRange, choosePageRange, picks, pageCounts }) => {
    const dpiInvalid = renderDpi(options) === null;
    const range = rangeSpans(options, pageRange);
    const rangeError = "error" in range && pageRange.trim() ? range.error : null;
    const pickedFiles = [...new Set(files)].filter((path) => picks[path]);
    return (
      <div className="workspace-field-grid">
        <fieldset className="workspace-field workspace-option-group workspace-field-wide">
          <legend>Format</legend>
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            aria-label="Format"
            value={options.format}
            onValueChange={(format) => { if (format) setOptions({ ...options, format }); }}
          >
            {imageFormats().map((format) => (
              <ToggleGroupItem key={format} value={format}>{formatLabel(format)}</ToggleGroupItem>
            ))}
          </ToggleGroup>
          <p className="workspace-note">{formatDetails[options.format]?.hint}</p>
        </fieldset>
        <fieldset className="workspace-field workspace-option-group workspace-field-wide">
          <legend>Resolution, dpi</legend>
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            aria-label="Resolution"
            value={options.resolution}
            onValueChange={(resolution) => { if (resolution) setOptions({ ...options, resolution: resolution as Resolution }); }}
          >
            {PRESET_RESOLUTIONS.map((dpi) => (
              <ToggleGroupItem key={dpi} value={dpi} aria-label={`${dpi} dpi`}>{dpi}</ToggleGroupItem>
            ))}
            <ToggleGroupItem value="custom">Custom</ToggleGroupItem>
          </ToggleGroup>
          {options.resolution === "custom" && (
            <div className="workspace-inline-field">
              <Label htmlFor="custom-dpi" className="sr-only">Custom resolution in dpi, {MIN_DPI} to {MAX_DPI}</Label>
              <Input
                id="custom-dpi"
                type="number"
                min={MIN_DPI}
                max={MAX_DPI}
                step={1}
                className="w-24"
                value={options.customDpi}
                aria-invalid={dpiInvalid || undefined}
                aria-describedby="custom-dpi-hint"
                onChange={(event) => setOptions({ ...options, customDpi: event.target.value })}
              />
              <span>dpi</span>
            </div>
          )}
          <p id="custom-dpi-hint" className="workspace-note" role={dpiInvalid ? "alert" : undefined}>{resolutionHint(options)}</p>
        </fieldset>
        <fieldset className="workspace-field workspace-option-group workspace-field-wide">
          <legend>Pages</legend>
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            aria-label="Pages"
            value={options.pages}
            onValueChange={(pages) => { if (pages) setOptions({ ...options, pages: pages as PagesMode }); }}
          >
            <ToggleGroupItem value="all">All pages</ToggleGroupItem>
            <ToggleGroupItem value="range">Range</ToggleGroupItem>
          </ToggleGroup>
          {options.pages === "range" && (
            <>
              <Label htmlFor="image-page-range" className="sr-only">Page range for every PDF</Label>
              <Input
                id="image-page-range"
                value={pageRange}
                aria-invalid={rangeError ? true : undefined}
                aria-describedby="image-page-range-hint"
                onChange={(event) => choosePageRange(event.target.value)}
                placeholder="1-3, 5, 8-"
              />
              {rangeError && <p className="text-sm text-destructive" role="alert">{rangeError}</p>}
              <p id="image-page-range-hint" className="workspace-note">Commas and dashes, like 1-3, 5, 8-. 8- means page 8 to the end.</p>
            </>
          )}
          <p className="workspace-note">
            {options.pages === "range"
              ? "One range applies to every PDF. Pages past a PDF's last page are left out, and a PDF with no pages left is skipped."
              : "Every page of every PDF. Choose Range to limit them, or select a PDF to pick its pages."}
          </p>
          {range.spans && files.length > 0 && (
            <p className="workspace-note" role="status">{rangeStatus({ options, files, pageRange, picks, pageCounts }, describeSpans(range.spans))}</p>
          )}
          {pickedFiles.map((path) => (
            <Badge key={path} variant="outline" className="max-w-full whitespace-normal">{fileName(path)} uses its own picks</Badge>
          ))}
        </fieldset>
      </div>
    );
  },
  exampleHint: "Page numbers are padded so names sort in page order. An existing name gets -1, -2 and so on.",
  exampleName: (inputPath, utility, { format }) => exampleOutputName(inputPath, "pdf_to_images", `${utility.capability.outputSuffix}-01`, format),
  request: (paths, outputLocation, { runId }, options, { pageRange, picks }) => invoke<ToolResult>("pdf_to_images", { runId,
    request: {
      paths,
      dpi: renderDpi(options),
      format: options.format,
      pageRange: options.pages === "range" ? pageRange.trim() : null,
      picks: [...new Set(paths)].filter((path) => picks[path]).map((path) => ({ path, pages: picks[path] })),
      outputLocation,
    },
  }),
};
