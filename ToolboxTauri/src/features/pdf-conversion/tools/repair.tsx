import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import type { ConversionTool } from "../tool";

export const repair: ConversionTool = {
  inspectsPdf: false,
  outputPreview: () => "Output preview unavailable. Export rebuilds each PDF and saves a fixed copy next to it.",
  OptionsPanel: () => <p className="workspace-note">Toolbox rebuilds each PDF and saves a fixed copy next to it.</p>,
  request: (paths, outputLocation, { runId }) => invoke<ToolResult>("repair_pdf", { runId,
    request: { paths, outputLocation },
  }),
};
