import { invoke } from "@tauri-apps/api/core";
import type { JobOutcome, ToolResult } from "../../../contracts";
import type { ConversionTool } from "../tool";
import { failedOutcome, parseToolResult } from "../../../jobOutcome";
import { isPastedHtml, pastedHtml } from "../../../components/pastedHtml";
import { ConvertToPdfOptionsPanel, convertToPdfIntakeCopy, convertToPdfRequest, defaultConvertToPdfOptions, type ConvertToPdfOptions } from "../../../views/ConvertToPdfControls";

/** Pasted markup has no file: it goes over as text with the path the user chose, and its result keeps the list's name for it. */
const convertPasted = async (path: string, options: ConvertToPdfOptions): Promise<JobOutcome[]> => {
  const item = pastedHtml(path);
  if (!item?.outputPath) return [failedOutcome(path, "invalidInput", "Choose where to save the pasted HTML, then try again.")];
  const outcomes = parseToolResult(await invoke<ToolResult>("convert_pasted_html", { request: { markup: item.markup, outputPath: item.outputPath, webPages: options.webPages } }));
  return outcomes.map((outcome) => ({ ...outcome, inputPath: path }));
};

export const convertToPdf: ConversionTool<ConvertToPdfOptions> = {
  initialOptions: defaultConvertToPdfOptions,
  inspectsPdf: false,
  pastesHtml: true,
  intakeCopy: convertToPdfIntakeCopy,
  outputPreview: () => "Output preview unavailable. Each file becomes its own PDF, and the originals stay as they are.",
  OptionsPanel: ({ options, setOptions, files, loading }) => (
    <ConvertToPdfOptionsPanel files={files} options={options} onChange={setOptions} disabled={loading} />
  ),
  request: async (paths, outputLocation, { runId }, options) => {
    const files = paths.filter((path) => !isPastedHtml(path));
    const pasted: JobOutcome[] = [];
    for (const path of paths.filter(isPastedHtml)) pasted.push(...await convertPasted(path, options));
    const converted = files.length > 0 ? parseToolResult(await invoke<ToolResult>("convert_to_pdf", { runId, request: convertToPdfRequest(files, outputLocation, options) })) : [];
    // Results follow the list's order; anything the backend reports beyond the list stays at the end.
    const all = [...converted, ...pasted];
    const at = (outcome: JobOutcome) => {
      const index = paths.indexOf(outcome.inputPath);
      return index < 0 ? paths.length : index;
    };
    return all.map((outcome, order) => ({ outcome, order })).sort((left, right) => at(left.outcome) - at(right.outcome) || left.order - right.order).map(({ outcome }) => outcome);
  },
};
