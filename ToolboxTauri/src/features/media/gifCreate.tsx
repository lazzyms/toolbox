import { useRef } from "react";
import { CircleAlert, TriangleAlert } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { fileName } from "../../components/intake";
import { formatBytes } from "../../shared/fileSize";
import { GifFrameStrip, type GifFrame } from "./GifFrameStrip";
import { GifPlayback, useFramePreviews } from "./GifPlayback";
import { estimateGifBytes, isLargeGif } from "./gifTiming";
import type { MediaPanelProps, MediaTool } from "./types";

export interface GifCreateOptions {
  /** The delay for every frame, in milliseconds, as typed. */
  delay: string;
  loop: boolean;
  /** Own delays, as typed, by the frame's list entry key, so a delay moves with its frame and a repeated file can differ. */
  ownDelays: Record<string, string>;
}

const MIN_DELAY_MS = 1;
const MAX_DELAY_MS = 60000;
const RANGE = `${MIN_DELAY_MS} to ${MAX_DELAY_MS}`;

/** A whole number of milliseconds from 1 to 60000, or null. */
const parseDelay = (text: string): number | null => {
  if (!/^\d+$/.test(text.trim())) return null;
  const delay = Number(text);
  return delay >= MIN_DELAY_MS && delay <= MAX_DELAY_MS ? delay : null;
};

const framesOf = (options: GifCreateOptions, files: string[], keys: string[]): GifFrame[] =>
  keys.map((key, index) => {
    const own = options.ownDelays[key];
    const text = own ?? options.delay;
    return { key, path: files[index], own: own !== undefined, delayMs: parseDelay(text) };
  });

const globalProblem = (options: GifCreateOptions) =>
  parseDelay(options.delay) === null ? `The delay for every frame must be a whole number from ${RANGE} ms.` : null;

const ownProblems = (frames: GifFrame[]) =>
  frames.flatMap((frame, index) => frame.own && frame.delayMs === null ? [`Frame ${index + 1}'s own delay must be a whole number from ${RANGE} ms.`] : []);

const listFrames = (numbers: number[]) =>
  numbers.length === 1 ? `Frame ${numbers[0]} has its own delay.`
    : `Frames ${numbers.slice(0, -1).join(", ")} and ${numbers[numbers.length - 1]} have their own delays.`;

const GifCreatePanel = ({ utility, options, setOptions, files, fileKeys, selectedFileIndex, selectFile, moveFile, removeFile, loading }: MediaPanelProps<GifCreateOptions>) => {
  const ownInput = useRef<HTMLInputElement>(null);
  const frames = framesOf(options, files, fileKeys);
  const supportsPreview = utility.capability.supportsPreview;
  const previews = useFramePreviews(files, supportsPreview);
  // Frame 1 sets the canvas, so its size and the frame count give the estimate before running.
  const canvas = frames[0] ? previews[frames[0].path] ?? null : null;
  const bytes = canvas ? estimateGifBytes(canvas, frames.length) : null;
  const large = bytes !== null && isLargeGif(bytes);
  const selected = frames[selectedFileIndex];
  const globalError = globalProblem(options);
  const ownErrors = ownProblems(frames);
  const totalMs = frames.every((frame) => frame.delayMs !== null) ? frames.reduce((sum, frame) => sum + (frame.delayMs ?? 0), 0) : null;
  const otherOwn = frames.flatMap((frame, index) => frame.own && index !== selectedFileIndex ? [index + 1] : []);
  const setOwnDelay = (key: string, delay: string | null) => setOptions((current) => {
    const { [key]: _, ...others } = current.ownDelays;
    return { ...current, ownDelays: delay === null ? others : { ...others, [key]: delay } };
  });

  return (
    <>
      <GifFrameStrip
        frames={frames}
        previews={previews}
        totalMs={totalMs}
        supportsPreview={supportsPreview}
        selectedIndex={selectedFileIndex}
        selectIndex={selectFile}
        moveFrame={moveFile}
        removeFrame={removeFile}
        disabled={loading}
        notice={large && bytes !== null && (
          <Card role="status" className="media-gif-large flex-row gap-2.5 px-3 py-2.5 text-xs">
            <TriangleAlert className="size-4 shrink-0" aria-hidden="true" />
            <p>This GIF will be about {formatBytes(bytes)}. Large GIFs are slow to open and share. Remove frames or add smaller images to reduce it.</p>
          </Card>
        )}
      >
        {supportsPreview && (
          <GifPlayback frames={frames} previews={previews} selectedIndex={selectedFileIndex} loop={options.loop} disabled={loading} />
        )}
      </GifFrameStrip>
      <div className="workspace-field">
        <Label htmlFor="gif-delay">Delay for every frame</Label>
        <div className="workspace-unit-input">
          <Input
            id="gif-delay"
            className="media-delay-input"
            type="number"
            inputMode="numeric"
            min={MIN_DELAY_MS}
            max={MAX_DELAY_MS}
            step={1}
            value={options.delay}
            aria-invalid={globalError !== null}
            aria-describedby={globalError ? "gif-delay-hint gif-delay-error" : "gif-delay-hint"}
            onChange={(event) => {
              const delay = event.target.value;
              setOptions((current) => ({ ...current, delay }));
            }}
          />
          <span id="gif-delay-hint">ms, {RANGE}</span>
        </div>
        {globalError && (
          <p id="gif-delay-error" role="alert" className="workspace-field-error">
            <CircleAlert aria-hidden="true" />
            <span>{globalError}</span>
          </p>
        )}
      </div>
      {selected && (
        <div className="workspace-field" role="group" aria-labelledby="gif-frame-heading">
          <span id="gif-frame-heading" className="workspace-panel-label">Frame {selectedFileIndex + 1}, {fileName(selected.path)}</span>
          <div className="media-check-field">
            <Checkbox
              id="gif-own-delay"
              checked={selected.own}
              onCheckedChange={(checked) => {
                setOwnDelay(selected.key, checked === true ? String(parseDelay(options.delay) ?? 100) : null);
                if (checked === true) requestAnimationFrame(() => ownInput.current?.focus());
              }}
            />
            <Label htmlFor="gif-own-delay">Own delay</Label>
          </div>
          {selected.own ? (
            <>
              <div className="workspace-unit-input">
                <Input
                  ref={ownInput}
                  className="media-delay-input"
                  type="number"
                  inputMode="numeric"
                  min={MIN_DELAY_MS}
                  max={MAX_DELAY_MS}
                  step={1}
                  aria-label={`Delay for frame ${selectedFileIndex + 1}, milliseconds`}
                  aria-invalid={selected.delayMs === null}
                  aria-describedby={ownErrors.length > 0 ? "gif-own-delay-error" : undefined}
                  value={options.ownDelays[selected.key]}
                  onChange={(event) => setOwnDelay(selected.key, event.target.value)}
                />
                <span>ms</span>
              </div>
              <p className="workspace-note">Clear Own delay to use the delay for every frame again.</p>
            </>
          ) : (
            <p className="workspace-note">
              {globalError ? "Uses the delay for every frame." : `Uses ${parseDelay(options.delay)} ms, the delay for every frame.`}
              {otherOwn.length > 0 && ` ${listFrames(otherOwn)}`}
            </p>
          )}
          {ownErrors.length > 0 && (
            <p id="gif-own-delay-error" role="alert" className="workspace-field-error">
              <CircleAlert aria-hidden="true" />
              <span>{ownErrors.join(" ")}</span>
            </p>
          )}
        </div>
      )}
      <div className="workspace-field" role="group" aria-labelledby="gif-loop-heading">
        <span id="gif-loop-heading" className="workspace-panel-label">Loop</span>
        <div className="media-check-field">
          <Switch id="gif-loop" checked={options.loop} onCheckedChange={(loop) => setOptions((current) => ({ ...current, loop }))} />
          <Label htmlFor="gif-loop">Loop forever</Label>
        </div>
        <p className="workspace-note">Off plays once and stops on the last frame.</p>
      </div>
      {canvas && bytes !== null && (
        <div className="workspace-field" role="group" aria-labelledby="gif-size-heading">
          <span id="gif-size-heading" className="workspace-panel-label">Size</span>
          <p className="workspace-panel-copy">{canvas.width} × {canvas.height}, from frame 1</p>
          <p className="workspace-note">Other frames are centred. Larger ones are scaled down to fit, never stretched. Padding is transparent.</p>
          {large ? (
            <p className="workspace-field-warning">
              <TriangleAlert aria-hidden="true" />
              <span>About {formatBytes(bytes)}. That&apos;s large for a GIF.</span>
            </p>
          ) : (
            <p className="workspace-note">About {formatBytes(bytes)}.</p>
          )}
        </div>
      )}
    </>
  );
};

export const gifCreateTool: MediaTool<GifCreateOptions> = {
  initialOptions: { delay: "100", loop: true, ownDelays: {} },
  Panel: GifCreatePanel,
  canRun: (options, files, keys) => globalProblem(options) === null && ownProblems(framesOf(options, files, keys)).length === 0,
  buildRequest: (options, { paths, keys, outputLocation, outputPath }) => ({
    command: "create_gif",
    request: {
      paths,
      frameDelayMs: Number(options.delay),
      // Own delays by frame position, in play order; the native side checks the range again.
      frameDelayOverrides: keys.flatMap((key, index) => key in options.ownDelays ? [{ index, delayMs: Number(options.ownDelays[key]) }] : []),
      loopForever: options.loop,
      outputLocation,
      outputPath,
    },
  }),
};
