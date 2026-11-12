import type { ReactNode } from "react";
import type { JobOutcome, OutputLocation, ToolCapability, ToolCommand, ToolDefinition } from "../../contracts";
import type { OutputGrouping } from "../../components/resultRow";
import type { IntakeRedirect } from "../../components/intake";

/** What the shared run hands every Media tool when it builds its request. */
export interface MediaRunInput {
  paths: string[];
  /** Each path's list entry key, in the same order; a file added twice has two keys. */
  keys: string[];
  outputLocation: OutputLocation;
  outputPath: string | null;
}

export interface MediaRequest {
  command: ToolCommand;
  request: Record<string, unknown>;
}

export interface MediaPanelProps<Options> {
  utility: ToolDefinition;
  options: Options;
  setOptions: (update: (current: Options) => Options) => void;
  files: string[];
  outputLocation: OutputLocation;
  loading: boolean;
  selectedFileIndex: number;
  selectFile: (index: number) => void;
  /** Each file's list entry key, parallel to `files`. */
  fileKeys: string[];
  /** Moves a file through the shared list's ordering. */
  moveFile: (index: number, toIndex: number) => void;
  /** Removes a file from the shared list, never from disk. */
  removeFile: (index: number) => void;
}

/**
 * One Media tool: its options panel and the request it sends.
 * The view keeps each tool's options, so they survive switching tools on the rail.
 */
export interface MediaTool<Options> {
  initialOptions: Options;
  Panel: (props: MediaPanelProps<Options>) => ReactNode;
  buildRequest: (options: Options, input: MediaRunInput) => MediaRequest;
  /** Whether the options allow a run, beyond having files and not running. */
  canRun?: (options: Options, files: string[], keys: string[]) => boolean;
  /** The run button's label when the shared run has none; defaults to the tool's short title. */
  runLabel?: string | ((options: Options, files: string[]) => string);
  /** Whether the shared file list offers reordering; defaults to true. */
  showFileOrdering?: boolean;
  /** How the shared run treats the inputs under these options; defaults to the registered capability. */
  capability?: (options: Options, registered: ToolCapability) => ToolCapability;
  /** The example output names the destination shows for an input; defaults to the registered suffix. */
  exampleName?: (options: Options, inputPath: string) => string | readonly string[];
  /** Shows a succeeded file's outputs as one row that expands into its parts. */
  outputGroup?: OutputGrouping;
  /** A line after the finished headline, such as a total across files. */
  resultsHint?: (results: JobOutcome[]) => string | null;
  /** Points a file the tool refuses at the tool that takes it, such as a GIF at Extract GIF Frames. */
  intakeRedirect?: (path: string) => IntakeRedirect | null;
}
