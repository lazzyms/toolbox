import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { CompressImagesRequest, ToolDefinition, ToolResult } from "../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { ShortcutHint } from "../components/ShortcutHint";
import { shortcutTarget } from "../shortcuts";

export const CompressFilesWorkspaceView = ({ utility }: { utility: ToolDefinition }) => {
  const [pdfQuality, setPdfQuality] = useState(80);
  const [imageQuality, setImageQuality] = useState(80);
  const [lossless, setLossless] = useState(false);
  const isPdf = utility.id === "pdf-compress";

  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="compress-files"
      utility={utility}
      onRun={(paths, outputLocation, { runId }) => isPdf
        ? invoke<ToolResult>("compress_pdf", { runId, request: { paths, quality: pdfQuality, outputLocation } })
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
                {isPdf
                  ? "Save a smaller copy of each selected PDF while keeping its page geometry."
                  : "Save a smaller copy of each selected image. The originals stay untouched."}
              </p>
            </div>
            {isPdf ? (
              <div className="workspace-field slider-field">
                <div className="slider-label-row">
                  <Label id="compression-quality-label">Compression quality</Label>
                  <output>{pdfQuality}%</output>
                </div>
                <Slider aria-label="Compression quality" aria-labelledby="compression-quality-label" min={1} max={100} value={[pdfQuality]} onValueChange={([value]) => value !== undefined && setPdfQuality(value)} />
              </div>
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
              aria-label={`Export ${utility.title}`}
              {...shortcutTarget("run")}
            >
              {runLabel ?? utility.shortTitle}
              <ShortcutHint id="run" />
            </Button>
          </CardContent>
        </Card>
      )}
    </ToolScaffold>
  );
};
