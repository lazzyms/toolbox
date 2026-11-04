import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import type { ConversionTool } from "../tool";

export const imagesToPdf: ConversionTool = {
  inspectsPdf: false,
  outputPreview: () => "Output preview unavailable. Export combines the selected images into one PDF in file order.",
  OptionsPanel: () => <p className="workspace-note">JPEGs go in as they are. Other images are stored without loss, and transparency is kept.</p>,
  request: (paths, outputLocation, { outputPath, runId }) => invoke<ToolResult>("images_to_pdf", { runId,
    request: { paths, outputLocation, outputPath },
  }),
};
