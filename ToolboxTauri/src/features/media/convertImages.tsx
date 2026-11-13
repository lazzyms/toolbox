import { Label } from "@/components/ui/label";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { exampleOutputName } from "../../components/RunDestination";
import { extensionOf, type IntakeRedirect } from "../../components/intake";
import type { ConvertImagesRequest, TiffCompression } from "../../contracts";
import { convertImagesRequest, convertTargets, defaultTiffCompression, tiffCompressions } from "../../registry/convertTargets";
import type { MediaPanelProps, MediaTool } from "./types";

export interface ConvertImagesOptions {
  format: ConvertImagesRequest["format"];
  tiffCompression: TiffCompression;
}

// Converting keeps one frame of a GIF, so Convert points GIFs at Extract GIF Frames instead.
const convertGifRedirect = (path: string): IntakeRedirect | null => extensionOf(path) === ".gif"
  ? { message: "Converting a GIF would drop its animation. Extract GIF Frames saves every frame.", toolId: "gif-extract" }
  : null;

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
          if (value) setOptions((current) => ({ ...current, format: value as ConvertImagesRequest["format"] }));
        }}
      >
        {convertTargets.map(({ format, name }) => <ToggleGroupItem key={format} value={format}>{name}</ToggleGroupItem>)}
      </ToggleGroup>
      <p id="image-target-format-hint" className="workspace-panel-copy">{convertTargets.find(({ format }) => format === options.format)?.hint}</p>
    </div>
    {options.format === "tiff" && (
      <div className="workspace-field">
        <Label id="tiff-compression-label">TIFF compression</Label>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          aria-labelledby="tiff-compression-label"
          aria-describedby="tiff-compression-hint"
          value={options.tiffCompression}
          onValueChange={(value) => {
            if (value) setOptions((current) => ({ ...current, tiffCompression: value as TiffCompression }));
          }}
        >
          {tiffCompressions.map(({ value, name }) => <ToggleGroupItem key={value} value={value}>{name}</ToggleGroupItem>)}
        </ToggleGroup>
        <p id="tiff-compression-hint" className="workspace-panel-copy">All three are lossless. LZW is the default. 16-bit sources stay 16-bit, and 8-bit sources stay 8-bit.</p>
      </div>
    )}
    <p className="workspace-note">Lossy results use the shared quality of 90, so a second run gives the same files. To choose a quality, use Compress.</p>
  </>
);

/** Convert Image Format sends the whole batch with one target; files already in that format are skipped natively. */
export const convertImagesTool: MediaTool<ConvertImagesOptions> = {
  initialOptions: { format: "jpg", tiffCompression: defaultTiffCompression },
  Panel: ConvertImagesPanel,
  buildRequest: ({ format, tiffCompression }, { paths, outputLocation }) => ({
    command: "convert_images",
    request: { ...convertImagesRequest(paths, format, tiffCompression, outputLocation) },
  }),
  showFileOrdering: false,
  exampleName: ({ format }, path) => exampleOutputName(path, "convert_images", "-converted", format),
  intakeRedirect: convertGifRedirect,
};
