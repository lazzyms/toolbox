import type { ReactNode } from "react";
import type { OutputLocation, ToolCommand, ToolDefinition } from "../../contracts";

/** What the shared run hands every Media tool when it builds its request. */
export interface MediaRunInput {
  paths: string[];
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
  canRun?: (options: Options, files: string[]) => boolean;
  /** The run button's label for this many files when the shared run has none; defaults to the tool's short title. */
  runLabel?: (fileCount: number) => string;
  /** Whether the shared file list offers reordering; defaults to true. */
  showFileOrdering?: boolean;
}
