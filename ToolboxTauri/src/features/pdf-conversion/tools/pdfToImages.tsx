import { invoke } from "@tauri-apps/api/core";
import { exampleOutputName } from "../../../components/RunDestination";
import type { ToolResult } from "../../../contracts";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { formatPageRange, inspectedPages, type ConversionTool } from "../tool";

type Options = { dpi: string; format: string };

export const pdfToImages: ConversionTool<Options> = {
  initialOptions: { dpi: "150", format: "jpg" },
  inspectsPdf: true,
  outputPreview: ({ dpi, format }, selectedPages) => {
    const pageSummary = selectedPages.length
      ? `${selectedPages.length === 1 ? "page" : "pages"} ${formatPageRange(selectedPages)}`
      : "the selected pages";
    const formatLabel = format === "jpg" ? "JPEG" : format.toUpperCase();
    return `Output preview unavailable. Export renders ${pageSummary} at ${dpi} DPI as ${formatLabel} image files.`;
  },
  OptionsPanel: ({ options, setOptions, pageRange, choosePageRange }) => (
    <div className="workspace-field-grid">
      <div className="workspace-field">
        <Label htmlFor="render-dpi">Resolution</Label>
        <NativeSelect id="render-dpi" aria-label="Render DPI" value={options.dpi} onChange={(event) => setOptions({ ...options, dpi: event.target.value })}>
          <option value="72">72 DPI</option>
          <option value="150">150 DPI</option>
          <option value="300">300 DPI</option>
        </NativeSelect>
      </div>
      <div className="workspace-field">
        <Label htmlFor="image-format">Format</Label>
        <NativeSelect id="image-format" aria-label="Output format" value={options.format} onChange={(event) => setOptions({ ...options, format: event.target.value })}>
          <option value="jpg">JPEG</option>
          <option value="png">PNG</option>
        </NativeSelect>
      </div>
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
  ),
  exampleName: (inputPath, utility, { format }) => exampleOutputName(inputPath, "pdf_to_images", utility.capability.outputSuffix, format),
  request: (paths, outputLocation, { runId }, { dpi, format }, selection) => {
    const pages = inspectedPages(selection);
    return invoke<ToolResult>("pdf_to_images", { runId,
      request: {
        paths,
        dpi: Number(dpi),
        format,
        pageRange: pages?.length ? formatPageRange(pages) : selection.pageRange || null,
        pages,
        outputLocation,
      },
    });
  },
};
