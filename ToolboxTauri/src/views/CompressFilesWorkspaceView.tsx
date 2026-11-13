import { Fragment, useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { CompressFilesRequest, CompressLevel, ToolDefinition, ToolResult } from "../contracts";
import { extensionOf, fileSizeLabel, parseEntryFacts, type EntryFacts, type IntakeCopy } from "../components/intake";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
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

// The estimator's answer for one file: rough sizes at each strength, or null when it has none.
type SizeEstimate = { originalBytes: number; low: number; medium: number; high: number };
const unknownFacts: EntryFacts = { pageCount: null, modifiedMs: null, sizeBytes: null };
const estimable = new Set([".pdf", ".jpg", ".jpeg", ".png", ".webp"]);
// Estimates read each file, so only a couple run at once and they never hold up the list.
const ESTIMATES_IN_FLIGHT = 2;
const estimateKey = (path: string, stripExif: boolean) => `${stripExif ? "strip" : "keep"}\0${path}`;
const parseEstimate = (value: unknown): SizeEstimate | null => {
  if (!value || typeof value !== "object") return null;
  const { originalBytes, low, medium, high } = value as Record<string, unknown>;
  const sizes = [originalBytes, low, medium, high];
  return sizes.every((size) => typeof size === "number" && Number.isFinite(size) && size >= 0)
    ? { originalBytes: originalBytes as number, low: low as number, medium: medium as number, high: high as number }
    : null;
};

const runLabelFor = (count: number) => count === 0 ? "Compress" : `Compress ${count} ${count === 1 ? "file" : "files"}`;

export const CompressFilesWorkspaceView = ({ utility }: { utility: ToolDefinition }) => {
  const [level, setLevel] = useState<CompressLevel>("medium");
  const [stripExif, setStripExif] = useState(false);
  const selected = levels.find(({ value }) => value === level) ?? levels[1];
  const [listedPaths, setListedPaths] = useState<readonly string[]>([]);
  const [facts, setFacts] = useState<ReadonlyMap<string, EntryFacts>>(() => new Map());
  // Keyed by EXIF choice and path, so a strength change only picks another level and never asks again.
  // A key that is present but undefined is being estimated; null means the file has no estimate.
  const [estimates, setEstimates] = useState<ReadonlyMap<string, SizeEstimate | null | undefined>>(() => new Map());
  const listChanged = useCallback((paths: readonly string[]) => setListedPaths(paths), []);

  // Every listed file shows its size. Files that can't be read show none, and each is asked about once.
  useEffect(() => {
    const unknown = [...new Set(listedPaths)].filter((path) => !facts.has(path));
    if (unknown.length === 0) return;
    let current = true;
    const remember = (described: ReadonlyMap<string, EntryFacts>) => {
      if (!current) return;
      setFacts((known) => new Map([...known, ...unknown.map((path) => [path, described.get(path) ?? unknownFacts] as const)]));
    };
    invoke("describe_intake_files", { paths: unknown })
      .then((value) => remember(parseEntryFacts(value)))
      .catch(() => remember(new Map()));
    return () => {
      current = false;
    };
  }, [listedPaths, facts]);

  // Estimates run in the background, a few at a time, for listed files of the types Compress takes.
  // The estimator itself returns none for an animated image, which the list flags as won't run.
  // A result for a file that has left the list is dropped; a failure shows no estimate.
  useEffect(() => {
    const listed = new Set(listedPaths);
    const pending = [...estimates.entries()].filter(([, estimate]) => estimate === undefined).length;
    const next = [...listed]
      .filter((path) => estimable.has(extensionOf(path)) && !estimates.has(estimateKey(path, stripExif)))
      .slice(0, Math.max(0, ESTIMATES_IN_FLIGHT - pending));
    if (next.length === 0) return;
    setEstimates((known) => new Map([...known, ...next.map((path) => [estimateKey(path, stripExif), undefined] as const)]));
    for (const path of next) {
      const key = estimateKey(path, stripExif);
      invoke("estimate_compressed_size", { path, stripExif })
        .then(parseEstimate, () => null)
        .then((estimate) => setEstimates((known) => {
          if (!known.has(key)) return known;
          const updated = new Map(known);
          updated.set(key, estimate);
          return updated;
        }));
    }
  }, [listedPaths, stripExif, estimates]);

  // Forget estimates for files that left the list, so a file added back is estimated afresh.
  useEffect(() => {
    const listed = new Set(listedPaths);
    setEstimates((known) => {
      const kept = [...known].filter(([key]) => listed.has(key.slice(key.indexOf("\0") + 1)));
      return kept.length === known.size ? known : new Map(kept);
    });
  }, [listedPaths]);

  // Only rows that will run reach here: skipped and won't-run rows show their badge instead.
  const estimateFor = (path: string) => {
    const key = estimateKey(path, stripExif);
    const estimate = estimates.get(key);
    if (estimate === null) return null;
    if (estimate === undefined) return <span className="file-selection-fact file-selection-estimate">Estimating…</span>;
    return <span className="file-selection-fact file-selection-estimate" data-estimated="true">About {fileSizeLabel(estimate[level])}</span>;
  };

  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="compress-files"
      utility={utility}
      emptyHint="PDF, JPEG, PNG and WebP. Mix them in one batch."
      intakeCopy={compressIntakeCopy}
      onFilesChange={listChanged}
      entryFacts={(path) => facts.get(path)}
      renderFileExtra={estimateFor}
      onRun={(paths, outputLocation, { runId }) =>
        invoke<ToolResult>("compress_files", { runId, request: { paths, level, stripExif, outputLocation } satisfies CompressFilesRequest })}
    >
      {({ files, run, loading, destination }) => (
        <Card className="workspace-control-panel py-0">
          <CardContent className="workspace-control-panel-content grid gap-4 p-4">
            <div>
              <h2 className="workspace-active-command">{utility.title}</h2>
              <p className="workspace-panel-copy">Save a smaller copy of each selected file. The originals stay untouched.</p>
            </div>
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
              <p className="workspace-note mt-0">Estimates in the list follow the strength. Estimates are rough. Real sizes show when the batch finishes.</p>
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
            {destination}
            <Button
              variant="default"
              disabled={loading || files.length === 0 || utility.capability.nativeAvailability !== "available"}
              onClick={run}
              className="workspace-primary-action"
              {...shortcutTarget("run")}
            >
              {runLabelFor(files.length)}
              <ShortcutHint id="run" />
            </Button>
          </CardContent>
        </Card>
      )}
    </ToolScaffold>
  );
};
