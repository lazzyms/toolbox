import type { JobOutcome, SucceededJobOutcome } from "../../contracts";
import { exampleOutputName } from "../../components/RunDestination";
import { fileName, type OutputGroup } from "../../components/resultRow";
import type { MediaTool } from "./types";

const isTimings = (path: string) => path.toLowerCase().endsWith(".json");

/** "spin-frame-001.png to -048.png": the first name, then what changes in the last one. */
export const frameRangeName = (first: string, last: string) => {
  if (first === last) return first;
  let common = 0;
  while (common < first.length && first[common] === last[common]) common += 1;
  const cut = first.lastIndexOf("-", common - 1);
  return cut < 0 ? `${first} to ${last}` : `${first} to ${last.slice(cut)}`;
};

const framesIn = (outcome: SucceededJobOutcome) => outcome.outputPaths.filter((path) => !isTimings(path));

const groupFrames = (outcome: SucceededJobOutcome): OutputGroup | null => {
  const timings = outcome.outputPaths.find(isTimings);
  const frames = framesIn(outcome);
  if (!timings || frames.length === 0) return null;
  const count = frames.length;
  const first = frames[0];
  return {
    hint: count === 1 ? "1 frame and a timings file. This GIF isn't animated." : `${count} frames and a timings file`,
    primary: timings,
    parts: [
      { kind: "file", name: fileName(timings), hint: "Each frame file and its delay", open: timings, reveal: timings },
      {
        kind: "set",
        name: frameRangeName(fileName(first), fileName(frames[count - 1])),
        hint: outcome.detail || (count === 1 ? "1 PNG" : `${count} PNGs`),
        open: null,
        reveal: first,
      },
    ],
  };
};

const framesSaved = (results: JobOutcome[]) => {
  const total = results.reduce((sum, outcome) => sum + (outcome.state === "succeeded" ? framesIn(outcome).length : 0), 0);
  if (total === 0) return null;
  return total === 1 ? "1 frame saved." : `${total} frames saved.`;
};

const OutputsPanel = () => (
  <section aria-labelledby="gif-extract-outputs">
    <h3 id="gif-extract-outputs" className="workspace-panel-label">You get, for each GIF</h3>
    <p className="workspace-panel-copy">
      Every frame as a full PNG, numbered so it sorts in order. Frames that store only changes are filled in, so each PNG looks like the frame on screen.
    </p>
    <p className="workspace-panel-copy">One timings file that lists each frame file and its delay in milliseconds.</p>
  </section>
);

/** Extracting frames has no options: each GIF splits into its frames and one timings file. */
export const gifExtractTool: MediaTool<Record<string, never>> = {
  initialOptions: {},
  Panel: OutputsPanel,
  buildRequest: (_options, { paths, outputLocation }) => ({
    command: "extract_gif_frames",
    request: { paths, outputLocation },
  }),
  exampleName: (path) => [
    exampleOutputName(path, "extract_gif_frames", "-frame-001"),
    exampleOutputName(path, "extract_gif_frames", "-timings", "json"),
  ],
  outputGroup: groupFrames,
  resultsHint: framesSaved,
};
