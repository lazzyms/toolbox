import { invoke } from "@tauri-apps/api/core";
import { exampleOutputName } from "../../../components/RunDestination";
import type { ToolResult } from "../../../contracts";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { capabilityFor } from "../../../registry/capabilities";
import { formatPageRange, inspectedPages, type ConversionTool } from "../tool";

type Resolution = "72" | "150" | "300" | "custom";
type Options = { resolution: Resolution; customDpi: string; format: string };

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

export const pdfToImages: ConversionTool<Options> = {
  initialOptions: { resolution: "150", customDpi: "200", format: "png" },
  inspectsPdf: true,
  outputPreview: (options, selectedPages) => {
    const pageSummary = selectedPages.length
      ? `${selectedPages.length === 1 ? "page" : "pages"} ${formatPageRange(selectedPages)}`
      : "the selected pages";
    const dpi = renderDpi(options);
    return `Output preview unavailable. Export renders ${pageSummary} at ${dpi ?? options.customDpi} dpi as ${formatLabel(options.format)} image files.`;
  },
  exportBlocked: (options) => renderDpi(options) === null,
  OptionsPanel: ({ options, setOptions, pageRange, choosePageRange }) => {
    const dpiInvalid = renderDpi(options) === null;
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
        <div className="workspace-field workspace-field-wide">
          <Label htmlFor="image-page-range">Pages</Label>
          <Input
            id="image-page-range"
            aria-label="Page range"
            value={pageRange}
            onChange={(event) => choosePageRange(event.target.value)}
            placeholder="All pages, or 1-3"
          />
        </div>
      </div>
    );
  },
  exampleHint: "Page numbers are padded so names sort in page order. An existing name gets -1, -2 and so on.",
  exampleName: (inputPath, utility, { format }) => exampleOutputName(inputPath, "pdf_to_images", `${utility.capability.outputSuffix}-01`, format),
  request: (paths, outputLocation, { runId }, options, selection) => {
    const pages = inspectedPages(selection);
    return invoke<ToolResult>("pdf_to_images", { runId,
      request: {
        paths,
        dpi: renderDpi(options),
        format: options.format,
        pageRange: pages?.length ? formatPageRange(pages) : selection.pageRange || null,
        pages,
        outputLocation,
      },
    });
  },
};
