import { Fragment, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { CompressFilesRequest, CompressImagesRequest, CompressLevel, ToolDefinition, ToolResult } from "../contracts";
import { extensionOf, type IntakeCopy } from "../components/intake";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { ShortcutHint } from "../components/ShortcutHint";
import { shortcutTarget } from "../shortcuts";

// What each strength does to each supported format, in the board's order.
const levels: { value: CompressLevel; name: string; summary: string; does: [format: string, effect: string][] }[] = [
  { value: "low", name: "Low", summary: "Least saving, detail stays", does: [["PDF", "Repacked without loss. Images untouched."], ["JPEG", "Re-saved at high quality."], ["PNG", "Lossless. Pixels unchanged."], ["WebP", "Lossless stays lossless. Lossy at high quality."]] },
  { value: "medium", name: "Medium", summary: "Good for sharing and email", does: [["PDF", "Images inside recompressed and scaled down."], ["JPEG", "Re-saved at medium quality."], ["PNG", "Lossless. Pixels unchanged."], ["WebP", "Lossless stays lossless. Lossy at medium quality."]] },
  { value: "high", name: "High", summary: "Smallest, photos look softer", does: [["PDF", "Images at lower quality and resolution."], ["JPEG", "Re-saved at low quality."], ["PNG", "Cut to 256 colours, dithered."], ["WebP", "All become lossy, at low quality."]] },
];

const exifHint = (on: boolean) => on
  ? "On: removes camera, date and GPS details from JPEG, PNG and WebP. Photos stay upright. PDFs aren't changed."
  : "Off: camera, date and GPS details stay in the copies.";

const NOT_COMPRESSIBLE = "Not a PDF or image. Compress takes PDF, JPEG, PNG and WebP.";
const otherImages = new Set([".gif", ".tif", ".tiff", ".bmp", ".avif"]);
const formatName = (path: string) => {
  const extension = extensionOf(path);
  return extension === ".webp" ? "WebP" : extension.slice(1).toUpperCase();
};
const fileCount = (count: number) => `${count} ${count === 1 ? "file" : "files"}`;

// Every file stays in the list. HEIC is sent and comes back Skipped with a pointer to Convert Image
// Format; other types and animated images are flagged and kept out of the run.
const compressIntakeCopy: IntakeCopy = {
  notAccepted: NOT_COMPRESSIBLE,
  keepsUnsupported: true,
  unsupported: (path) => otherImages.has(extensionOf(path)) ? `${formatName(path)} isn't supported. Compress takes PDF, JPEG, PNG and WebP.` : NOT_COMPRESSIBLE,
  willSkip: (path) => `${formatName(path)} isn't compressed, so it will be skipped. Convert it to JPEG first.`,
  animated: (path) => `Animated ${formatName(path)}. Only still images can be compressed.`,
  flaggedNotice: (count, skipped) => {
    const parts = [skipped > 0 ? `${skipped} will be skipped` : null, count > skipped ? `${count - skipped} would fail` : null];
    return `${fileCount(count)} won't be compressed: ${parts.filter(Boolean).join(", ")}.`;
  },
  countHint: (flagged) => flagged > 0 ? `${flagged} won't be compressed` : "",
};

const runLabelFor = (count: number) => count === 0 ? "Compress" : `Compress ${count} ${count === 1 ? "file" : "files"}`;

export const CompressFilesWorkspaceView = ({ utility }: { utility: ToolDefinition }) => {
  const [level, setLevel] = useState<CompressLevel>("medium");
  const [stripExif, setStripExif] = useState(false);
  const [imageQuality, setImageQuality] = useState(80);
  const [lossless, setLossless] = useState(false);
  const isCompress = utility.id === "compress-files";
  const selected = levels.find(({ value }) => value === level) ?? levels[1];

  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="compress-files"
      utility={utility}
      emptyHint={isCompress ? "PDF, JPEG, PNG and WebP. Mix them in one batch." : undefined}
      intakeCopy={isCompress ? compressIntakeCopy : undefined}
      onRun={(paths, outputLocation, { runId }) => isCompress
        ? invoke<ToolResult>("compress_files", { runId, request: { paths, level, stripExif, outputLocation } satisfies CompressFilesRequest })
        : invoke<ToolResult>("compress_images", { runId,
          request: { paths, quality: imageQuality, lossless, outputLocation } satisfies CompressImagesRequest,
        })}
    >
      {({ files, run, loading, destination, runLabel }) => (
        <Card className="workspace-control-panel py-0">
          <CardContent className="workspace-control-panel-content grid gap-4 p-4">
            <div>
              <h2 className="workspace-active-command">{utility.title}</h2>
              <p className="workspace-panel-copy">
                {isCompress
                  ? "Save a smaller copy of each selected file. The originals stay untouched."
                  : "Save a smaller copy of each selected image. The originals stay untouched."}
              </p>
            </div>
            {isCompress ? (
              <>
                <div className="workspace-field">
                  <Label id="compress-strength-label">Strength</Label>
                  <RadioGroup aria-labelledby="compress-strength-label" className="gap-1.5" value={level} onValueChange={(value) => setLevel(value as CompressLevel)}>
                    {levels.map(({ value, name, summary }) => (
                      <span key={value} className="workspace-check">
                        <RadioGroupItem id={`compress-strength-${value}`} value={value} />
                        <Label htmlFor={`compress-strength-${value}`} className="gap-2">
                          <span>{name}</span>
                          <span className="workspace-panel-copy mt-0">{summary}</span>
                        </Label>
                      </span>
                    ))}
                  </RadioGroup>
                </div>
                <div className="workspace-field">
                  <span className="workspace-panel-label" id="compress-effect-label">What {selected.name} does</span>
                  <dl className="workspace-field-grid m-0" aria-labelledby="compress-effect-label">
                    {selected.does.map(([format, effect]) => (
                      <Fragment key={format}>
                        <dt>{format}</dt>
                        <dd className="m-0 text-foreground">{effect}</dd>
                      </Fragment>
                    ))}
                  </dl>
                  <p className="workspace-note mt-0">Text, transparency and colour profiles always stay.</p>
                </div>
                <div className="workspace-field">
                  <div className="workspace-field-heading">
                    <Label htmlFor="compress-strip-exif">Strip EXIF</Label>
                    <Switch id="compress-strip-exif" aria-describedby="compress-strip-exif-hint" checked={stripExif} onCheckedChange={setStripExif} />
                  </div>
                  <p id="compress-strip-exif-hint" className="workspace-note mt-0">{exifHint(stripExif)}</p>
                </div>
              </>
            ) : (
              <>
                <div className="workspace-field">
                  <div className="workspace-field-heading"><Label id="image-quality-label">Quality</Label><output>{imageQuality}%</output></div>
                  <Slider aria-labelledby="image-quality-label" min={1} max={100} value={[imageQuality]} onValueChange={([value]) => value !== undefined && setImageQuality(value)} />
                </div>
                <div className="workspace-check"><Label htmlFor="image-lossless">Preserve original pixels</Label><Switch id="image-lossless" checked={lossless} onCheckedChange={setLossless} /></div>
              </>
            )}
            {destination}
            <Button
              variant="default"
              disabled={loading || files.length === 0 || utility.capability.nativeAvailability !== "available"}
              onClick={run}
              className="workspace-primary-action"
              aria-label={isCompress ? undefined : `Export ${utility.title}`}
              {...shortcutTarget("run")}
            >
              {isCompress ? runLabelFor(files.length) : runLabel ?? utility.shortTitle}
              <ShortcutHint id="run" />
            </Button>
          </CardContent>
        </Card>
      )}
    </ToolScaffold>
  );
};
