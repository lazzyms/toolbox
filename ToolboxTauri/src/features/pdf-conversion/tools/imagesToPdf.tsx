import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ImagePreview, ToolResult } from "../../../contracts";
import { Card, CardContent } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { ConversionTool, OptionsPanelProps } from "../tool";

type PageSize = "matchImage" | "a4" | "letter";
type Margin = "none" | "small" | "large";
type Fit = "fit" | "fill";
type Orientation = "automatic" | "portrait" | "landscape";
type Options = { pageSize: PageSize; margin: Margin; fit: Fit; orientation: Orientation };

// Portrait paper in points and margins in points per side; the native kit
// uses the same values (src-tauri/src/kit/pdf/remaining.rs).
const paper = { a4: { name: "A4", short: 595.2756, long: 841.8898 }, letter: { name: "Letter", short: 612, long: 792 } } as const;
const marginPoints: Record<Margin, number> = { none: 0, small: 36, large: 72 };
const marginPhrase: Record<Margin, string> = { none: "no margin", small: "a small margin", large: "a large margin" };

/** Margin, fit and orientation travel only with A4 and Letter. */
const layoutRequest = ({ pageSize, margin, fit, orientation }: Options) =>
  pageSize === "matchImage" ? { pageSize } : { pageSize, margin, fit, orientation };

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

const PagePreview = ({ options, sheet, path, page }: { options: Options; sheet: (typeof paper)[keyof typeof paper]; path: string | undefined; page: number }) => {
  const [preview, setPreview] = useState<{ path: string; value: ImagePreview | null } | null>(null);
  useEffect(() => {
    if (!path) return;
    let current = true;
    void invoke<ImagePreview>("inspect_image_preview", { request: { path } })
      .then((value) => { if (current) setPreview({ path, value: value && typeof value.width === "number" ? value : null }); })
      .catch(() => { if (current) setPreview({ path, value: null }); });
    return () => { current = false; };
  }, [path]);
  const image = preview && preview.path === path ? preview.value : null;
  const landscape = options.orientation === "landscape"
    || (options.orientation === "automatic" && image !== null && image.width > image.height);
  const orientationKnown = options.orientation !== "automatic" || image !== null;
  const [pageWidth, pageHeight] = landscape ? [sheet.long, sheet.short] : [sheet.short, sheet.long];
  const scale = Math.min(200 / pageWidth, 130 / pageHeight);
  const inset = marginPoints[options.margin] * scale;
  const caption = path
    ? `Page ${page}, ${fileName(path)} on ${sheet.name}${orientationKnown ? ` ${landscape ? "landscape" : "portrait"}` : ""} with ${marginPhrase[options.margin]}. Select another page to preview it.`
    : "Add images to preview a page.";
  return (
    <figure className="images-to-pdf-preview">
      <span className="workspace-panel-label">Preview</span>
      <div className="images-to-pdf-preview-stage" aria-hidden="true">
        <div className="images-to-pdf-paper" data-orientation={landscape ? "landscape" : "portrait"} style={{ width: pageWidth * scale, height: pageHeight * scale, padding: inset }}>
          {image && <img src={image.dataUrl} alt="" data-fit={options.fit} />}
        </div>
      </div>
      <figcaption className="workspace-note">{caption}</figcaption>
    </figure>
  );
};

const ImagesToPdfOptions = ({ options, setOptions, files, selectedFileIndex }: OptionsPanelProps<Options>) => {
  const choose = <Key extends keyof Options>(key: Key) => (value: string) => {
    if (value) setOptions({ ...options, [key]: value as Options[Key] });
  };
  const sheet = options.pageSize === "matchImage" ? null : paper[options.pageSize];
  return (
    <div className="grid gap-4">
      <fieldset className="images-to-pdf-option">
        <legend className="workspace-panel-label">Page size</legend>
        <ToggleGroup type="single" variant="outline" size="sm" aria-label="Page size" value={options.pageSize} onValueChange={choose("pageSize")}>
          <ToggleGroupItem value="matchImage">Match image</ToggleGroupItem>
          <ToggleGroupItem value="a4">A4</ToggleGroupItem>
          <ToggleGroupItem value="letter">Letter</ToggleGroupItem>
        </ToggleGroup>
        {!sheet && <p className="workspace-note">Each page is the size of its image. Margin, fit and orientation appear for A4 and Letter.</p>}
      </fieldset>
      {sheet && (
        <Card className="images-to-pdf-layout py-0" role="group" aria-label={`${sheet.name} layout`}>
          <CardContent className="grid gap-4 p-3.5">
            <fieldset className="images-to-pdf-option">
              <legend className="workspace-panel-label">Orientation</legend>
              <ToggleGroup type="single" variant="outline" size="sm" aria-label="Orientation" value={options.orientation} onValueChange={choose("orientation")}>
                <ToggleGroupItem value="automatic">Automatic</ToggleGroupItem>
                <ToggleGroupItem value="portrait">Portrait</ToggleGroupItem>
                <ToggleGroupItem value="landscape">Landscape</ToggleGroupItem>
              </ToggleGroup>
              <p className="workspace-note">Automatic turns each page to suit its image.</p>
            </fieldset>
            <fieldset className="images-to-pdf-option">
              <legend className="workspace-panel-label">Fit</legend>
              <RadioGroup aria-label="Fit" className="gap-1.5" value={options.fit} onValueChange={choose("fit")}>
                <span className="settings-radio">
                  <RadioGroupItem id="images-to-pdf-fit" value="fit" />
                  <Label htmlFor="images-to-pdf-fit">Fit within the page</Label>
                </span>
                <span className="settings-radio">
                  <RadioGroupItem id="images-to-pdf-fill" value="fill" />
                  <Label htmlFor="images-to-pdf-fill">Fill the page, trimming edges</Label>
                </span>
              </RadioGroup>
            </fieldset>
            <fieldset className="images-to-pdf-option">
              <legend className="workspace-panel-label">Margin</legend>
              <ToggleGroup type="single" variant="outline" size="sm" aria-label="Margin" value={options.margin} onValueChange={choose("margin")}>
                <ToggleGroupItem value="none">None</ToggleGroupItem>
                <ToggleGroupItem value="small">Small</ToggleGroupItem>
                <ToggleGroupItem value="large">Large</ToggleGroupItem>
              </ToggleGroup>
            </fieldset>
            <PagePreview options={options} sheet={sheet} path={files[selectedFileIndex] ?? files[0]} page={(files[selectedFileIndex] ? selectedFileIndex : 0) + 1} />
          </CardContent>
        </Card>
      )}
      <p className="workspace-note">JPEGs go in as they are. Other images are stored without loss, and transparency is kept. A multi-page TIFF adds one page per TIFF page.</p>
    </div>
  );
};

export const imagesToPdf: ConversionTool<Options> = {
  initialOptions: { pageSize: "matchImage", margin: "small", fit: "fit", orientation: "automatic" },
  inspectsPdf: false,
  sortOptions: [
    { value: "added", label: "Order added" },
    { value: "nameAsc", label: "Name" },
    { value: "modifiedAsc", label: "Modified date" },
  ],
  outputPreview: () => "Output preview unavailable. Export combines the selected images into one PDF in file order.",
  OptionsPanel: ImagesToPdfOptions,
  request: (paths, outputLocation, { outputPath, runId }, options) => invoke<ToolResult>("images_to_pdf", { runId,
    request: { paths, outputLocation, outputPath, ...layoutRequest(options) },
  }),
};
