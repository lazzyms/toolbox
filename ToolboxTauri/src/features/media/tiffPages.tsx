import { TiffPageInspector, isCurrentTiffInspection, type TiffPagesState } from "./TiffPageInspector";
import type { MediaPanelProps, MediaTool } from "./types";

const TiffPagesPanel = ({ utility, options, setOptions, files }: MediaPanelProps<TiffPagesState>) => (
  <TiffPageInspector files={files} supportsPreview={utility.capability.supportsPreview} state={options} setState={setOptions} />
);

export const tiffPagesTool: MediaTool<TiffPagesState> = {
  initialOptions: { inspection: { kind: "idle" }, selectedIndex: 0 },
  Panel: TiffPagesPanel,
  // A run waits until the pages of the current files have been read.
  canRun: ({ inspection }, files) => isCurrentTiffInspection(inspection, files),
  // The inspector orders pages, so the shared file list does not.
  showFileOrdering: false,
  buildRequest: ({ inspection }, { paths, outputLocation, outputPath }) => ({
    command: "process_tiff_pages",
    request: {
      paths,
      pages: isCurrentTiffInspection(inspection, paths)
        ? inspection.frames.map(({ path, page }) => ({ path, page }))
        : [],
      outputLocation,
      outputPath,
    },
  }),
};
