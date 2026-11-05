import type { MediaTool } from "./types";

/** Extracting frames has no options: each GIF splits into its frames. */
export const gifExtractTool: MediaTool<Record<string, never>> = {
  initialOptions: {},
  Panel: () => null,
  buildRequest: (_options, { paths, outputLocation }) => ({
    command: "extract_gif_frames",
    request: { paths, outputLocation },
  }),
};
