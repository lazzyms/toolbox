import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import { ConvertToPdfOptionsPanel, convertToPdfIntakeCopy, convertToPdfRequest, defaultConvertToPdfOptions, type ConvertToPdfOptions } from "../../../views/ConvertToPdfControls";
import type { ConversionTool } from "../tool";

export const convertToPdf: ConversionTool<ConvertToPdfOptions> = {
  initialOptions: defaultConvertToPdfOptions,
  inspectsPdf: false,
  outputPreview: () => "Output preview unavailable. Each file becomes its own PDF, and the originals stay as they are.",
  intakeCopy: convertToPdfIntakeCopy,
  OptionsPanel: ({ files, options, setOptions, loading }) => <ConvertToPdfOptionsPanel files={files} options={options} onChange={setOptions} disabled={loading} />,
  request: (paths, outputLocation, { runId }, options) => invoke<ToolResult>("convert_to_pdf", { runId,
    request: convertToPdfRequest(paths, outputLocation, options),
  }),
};
