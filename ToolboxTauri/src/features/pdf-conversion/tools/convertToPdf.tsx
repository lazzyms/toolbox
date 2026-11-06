import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import type { ConversionTool } from "../tool";
import { ConvertToPdfOptionsPanel, convertToPdfIntakeCopy, convertToPdfRequest, defaultConvertToPdfOptions, type ConvertToPdfOptions } from "../../../views/ConvertToPdfControls";

export const convertToPdf: ConversionTool<ConvertToPdfOptions> = {
  initialOptions: defaultConvertToPdfOptions,
  inspectsPdf: false,
  intakeCopy: convertToPdfIntakeCopy,
  outputPreview: () => "Output preview unavailable. Each file becomes its own PDF, and the originals stay as they are.",
  OptionsPanel: ({ options, setOptions, files, loading }) => (
    <ConvertToPdfOptionsPanel files={files} options={options} onChange={setOptions} disabled={loading} />
  ),
  request: (paths, outputLocation, { runId }, options) =>
    invoke<ToolResult>("convert_to_pdf", { runId, request: convertToPdfRequest(paths, outputLocation, options) }),
};
