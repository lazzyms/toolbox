import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import type { ConversionTool } from "../tool";

export const imagesToPdf: ConversionTool = {
  inspectsPdf: false,
  outputPreview: () => "Output preview unavailable. Export combines the selected images into one PDF in file order.",
  OptionsPanel: () => <p className="workspace-note">Images are combined in the order they appear in the selected list.</p>,
  request: (paths, outputLocation, { outputPath, runId }) => invoke<ToolResult>("images_to_pdf", { runId,
    request: { paths, outputLocation, outputPath },
  }),
};
