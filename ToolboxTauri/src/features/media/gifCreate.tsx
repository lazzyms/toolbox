import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { GifFrameStrip } from "./GifFrameStrip";
import type { MediaPanelProps, MediaTool } from "./types";

export interface GifCreateOptions {
  delay: number;
  loop: boolean;
}

const GifCreatePanel = ({ utility, options, setOptions, files, selectedFileIndex, selectFile }: MediaPanelProps<GifCreateOptions>) => (
  <>
    <GifFrameStrip files={files} supportsPreview={utility.capability.supportsPreview} selectedIndex={selectedFileIndex} selectIndex={selectFile} />
    <div className="workspace-field-grid">
      <div className="workspace-field">
        <Label htmlFor="frame-delay">Frame delay</Label>
        <Input
          id="frame-delay"
          type="number"
          aria-label="Frame delay in milliseconds"
          min="1"
          max="60000"
          value={options.delay}
          onChange={(event) => {
            const delay = Number(event.target.value);
            setOptions((current) => ({ ...current, delay }));
          }}
        />
      </div>
      <div className="media-toggle-field">
        <Label htmlFor="gif-loop">Loop animation</Label>
        <Switch id="gif-loop" checked={options.loop} onCheckedChange={(loop) => setOptions((current) => ({ ...current, loop }))} />
      </div>
    </div>
  </>
);

export const gifCreateTool: MediaTool<GifCreateOptions> = {
  initialOptions: { delay: 100, loop: true },
  Panel: GifCreatePanel,
  buildRequest: ({ delay, loop }, { paths, outputLocation, outputPath }) => ({
    command: "create_gif",
    request: { paths, frameDelayMs: delay, loopForever: loop, outputLocation, outputPath },
  }),
};
