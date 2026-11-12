import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Pause, Play } from "lucide-react";
import type { ImagePreview } from "../../contracts";
import { Button } from "@/components/ui/button";
import { Slider } from "@/components/ui/slider";
import { fileName } from "../../components/intake";
import type { GifFrame } from "./GifFrameStrip";
import { playedDelayMs } from "./gifTiming";

/** Each distinct path's preview, or null when it can't be previewed; a path is fetched once. */
export const useFramePreviews = (paths: string[], enabled: boolean) => {
  const [previews, setPreviews] = useState<Record<string, ImagePreview | null>>({});
  const unique = [...new Set(paths)];
  const pathKey = unique.join("\u0000");

  useEffect(() => {
    if (!pathKey || !enabled) {
      setPreviews({});
      return;
    }
    let current = true;
    setPreviews({});
    void Promise.all(
      unique.map(async (path) => {
        try {
          return [path, await invoke<ImagePreview>("inspect_image_preview", { request: { path } })] as const;
        } catch {
          return [path, null] as const;
        }
      }),
    ).then((entries) => {
      if (current) setPreviews(Object.fromEntries(entries));
    });
    return () => {
      current = false;
    };
    // The path key stands for the paths, so a new array with the same paths does not refetch.
  }, [pathKey, enabled]);

  return previews;
};

const BOX = { width: 240, height: 180 };

/** The canvas drawn at most 240 × 180, keeping its shape. */
const boxFor = (canvas: { width: number; height: number }) => {
  const scale = Math.min(BOX.width / canvas.width, BOX.height / canvas.height);
  return { width: Math.round(canvas.width * scale), height: Math.round(canvas.height * scale) };
};

/** Where a frame sits on the canvas, as the GIF places it: centred, and scaled down to fit but never up. */
const placement = (frame: { width: number; height: number }, canvas: { width: number; height: number }) => {
  const scale = Math.min(1, canvas.width / frame.width, canvas.height / frame.height);
  return {
    width: `${((frame.width * scale) / canvas.width) * 100}%`,
    height: `${((frame.height * scale) / canvas.height) * 100}%`,
  };
};

/**
 * Plays the frames on frame 1's canvas, in the current order, with each frame's
 * delay as the GIF stores it and the loop setting. It only plays when asked.
 */
export const GifPlayback = ({ frames, previews, selectedIndex, loop, disabled }: {
  frames: GifFrame[];
  previews: Record<string, ImagePreview | null>;
  selectedIndex: number;
  loop: boolean;
  disabled: boolean;
}) => {
  const [position, setPosition] = useState(selectedIndex);
  const [playing, setPlaying] = useState(false);
  const count = frames.length;
  const shown = Math.max(0, Math.min(position, count - 1));
  const frame = frames[shown];
  const playable = !disabled && count > 0 && frames.every((each) => each.delayMs !== null);

  // Selecting a frame, which a move also does, shows that frame.
  useEffect(() => setPosition(selectedIndex), [selectedIndex]);
  useEffect(() => {
    if (!playable) setPlaying(false);
  }, [playable]);
  useEffect(() => {
    if (!playing || !frame || frame.delayMs === null) return;
    const timer = window.setTimeout(() => {
      if (shown + 1 < count) setPosition(shown + 1);
      else if (loop) setPosition(0);
      else setPlaying(false);
    }, playedDelayMs(frame.delayMs));
    return () => window.clearTimeout(timer);
  }, [playing, shown, count, loop, frame?.key, frame?.delayMs]);

  const canvasPreview = frames[0] ? previews[frames[0].path] : undefined;
  if (!frame || !canvasPreview) return null;
  const canvas = { width: canvasPreview.width, height: canvasPreview.height };
  const preview = previews[frame.path];
  const where = `Frame ${shown + 1} of ${count}`;

  return (
    <div className="media-gif-playback">
      <div
        className="media-gif-canvas"
        role="img"
        aria-label={`Preview, ${where}, ${fileName(frame.path)}`}
        data-frame-key={frame.key}
        style={boxFor(canvas)}
      >
        {preview && <img src={preview.dataUrl} alt="" draggable={false} style={placement(preview, canvas)} />}
      </div>
      <div className="media-gif-playback-controls">
        <span className="workspace-panel-label">Preview</span>
        <div className="media-gif-playback-row">
          <Button
            variant="outline"
            size="sm"
            disabled={!playable}
            onClick={() => {
              if (playing) {
                setPlaying(false);
                return;
              }
              // Playing once from the last frame starts again from the first.
              if (!loop && shown === count - 1) setPosition(0);
              setPlaying(true);
            }}
          >
            {playing ? <Pause aria-hidden="true" /> : <Play aria-hidden="true" />}
            {playing ? "Pause" : "Play"}
          </Button>
          <span className="workspace-note">{where}</span>
        </div>
        <Slider
          aria-label="Preview frame"
          min={1}
          max={Math.max(count, 1)}
          step={1}
          value={[shown + 1]}
          disabled={disabled || count < 2}
          onValueChange={([value]) => setPosition(value - 1)}
        />
        <p className="workspace-note">
          Shows the GIF at {canvas.width} × {canvas.height} with your delays and loop. It doesn&apos;t play on its own when Reduce motion is on.
        </p>
      </div>
    </div>
  );
};
