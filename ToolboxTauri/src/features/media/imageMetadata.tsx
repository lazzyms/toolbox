import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { ImageMetadataInspection } from "../../contracts";
import { ImageMetadataReport } from "../../views/ImageMetadataReport";
import type { MediaPanelProps, MediaTool } from "./types";

export interface ImageMetadataOptions {
  mode: "inspect" | "strip";
}

const ImageMetadataPanel = ({ options, setOptions, files, outputLocation, loading }: MediaPanelProps<ImageMetadataOptions>) => {
  // The report belongs to this panel, so it clears when another tool is chosen.
  const [report, setReport] = useState<ImageMetadataInspection[]>([]);
  const [inspectError, setInspectError] = useState<string>();
  const [inspecting, setInspecting] = useState(false);

  const inspectMetadata = async () => {
    setInspecting(true);
    setInspectError(undefined);
    try {
      setReport(await invoke<ImageMetadataInspection[]>("inspect_image_metadata", { request: { paths: files, outputLocation } }));
    } catch (error) {
      setReport([]);
      setInspectError(String(error));
    } finally {
      setInspecting(false);
    }
  };

  // Only files still in the list keep their card, so removing a file also drops its report.
  const inspections = report.filter((inspection) => files.includes(inspection.path));
  return (
    <>
      <ToggleGroup
        type="single"
        variant="outline"
        size="sm"
        aria-label="Metadata mode"
        value={options.mode}
        onValueChange={(value) => {
          if (value) setOptions(() => ({ mode: value as ImageMetadataOptions["mode"] }));
        }}
      >
        <ToggleGroupItem value="inspect">Inspect metadata</ToggleGroupItem>
        <ToggleGroupItem value="strip">Remove metadata in a new copy</ToggleGroupItem>
      </ToggleGroup>
      <Button
        variant="ghost"
        size="sm"
        disabled={loading || inspecting || files.length === 0}
        onClick={() => void inspectMetadata()}
      >
        Inspect metadata
      </Button>
      {(inspections.length > 0 || inspectError) && <ImageMetadataReport inspections={inspections} error={inspectError} />}
    </>
  );
};

export const imageMetadataTool: MediaTool<ImageMetadataOptions> = {
  initialOptions: { mode: "strip" },
  Panel: ImageMetadataPanel,
  // Inspecting has its own button; the run only removes metadata.
  canRun: ({ mode }) => mode !== "inspect",
  runLabel: "Remove metadata",
  buildRequest: (_options, { paths, outputLocation }) => ({
    command: "image_metadata",
    request: { paths, outputLocation },
  }),
};
