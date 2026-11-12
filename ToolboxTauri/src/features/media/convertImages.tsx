import { Label } from "@/components/ui/label";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { exampleOutputName } from "../../components/RunDestination";
import type { ConvertImagesRequest } from "../../contracts";
import type { MediaPanelProps, MediaTool } from "./types";

export interface ConvertImagesOptions {
  format: ConvertImagesRequest["format"];
}

const convertTargets: { format: ConvertImagesRequest["format"]; name: string; hint: string }[] = [
  { format: "jpg", name: "JPEG", hint: "Transparent areas become white, because JPEG has no transparency." },
  { format: "png", name: "PNG", hint: "Lossless. Transparency is kept." },
  { format: "webp", name: "WebP", hint: "Lossy. Transparency is kept." },
  { format: "heic", name: "HEIC", hint: "Saved as 8-bit. Transparency is kept." },
];

const ConvertImagesPanel = ({ options, setOptions }: MediaPanelProps<ConvertImagesOptions>) => (
  <>
    <div className="workspace-field">
      <Label id="image-target-format-label">Convert to</Label>
      <ToggleGroup
        type="single"
        variant="outline"
        size="sm"
        aria-labelledby="image-target-format-label"
        aria-describedby="image-target-format-hint"
        value={options.format}
        onValueChange={(value) => {
          if (value) setOptions(() => ({ format: value as ConvertImagesRequest["format"] }));
        }}
      >
        {convertTargets.map(({ format, name }) => <ToggleGroupItem key={format} value={format}>{name}</ToggleGroupItem>)}
      </ToggleGroup>
      <p id="image-target-format-hint" className="workspace-panel-copy">{convertTargets.find(({ format }) => format === options.format)?.hint}</p>
    </div>
    <p className="workspace-note">Lossy results use the shared quality of 90, so a second run gives the same files. To choose a quality, use Compress.</p>
  </>
);

/** Convert Image Format sends the whole batch with one target; files already in that format are skipped natively. */
export const convertImagesTool: MediaTool<ConvertImagesOptions> = {
  initialOptions: { format: "jpg" },
  Panel: ConvertImagesPanel,
  buildRequest: ({ format }, { paths, outputLocation }) => ({
    command: "convert_images",
    request: { paths, format, outputLocation } satisfies ConvertImagesRequest,
  }),
  showFileOrdering: false,
  exampleName: ({ format }, path) => exampleOutputName(path, "convert_images", "-converted", format),
};
