import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import type { MediaPanelProps, MediaTool } from "./types";

export interface IconSetOptions {
  preset: string;
  sizes: number[];
}

const iconSizesByPreset: Record<string, number[]> = {
  macos: [16, 32, 128, 256, 512, 1024],
  favicon: [16, 32, 48, 64, 128, 256],
  ios: [20, 29, 40, 60, 76, 83, 1024],
  android: [48, 72, 96, 144, 192, 512],
  custom: [16, 32, 64, 128, 256, 512],
};

const IconSetPanel = ({ options, setOptions }: MediaPanelProps<IconSetOptions>) => (
  <>
    <div className="workspace-field">
      <Label htmlFor="icon-preset">Icon preset</Label>
      <NativeSelect
        id="icon-preset"
        aria-label="Icon preset"
        value={options.preset}
        onChange={(event) => {
          const preset = event.target.value;
          setOptions(() => ({ preset, sizes: iconSizesByPreset[preset] ?? iconSizesByPreset.custom }));
        }}
      >
        <option value="macos">macOS</option>
        <option value="favicon">Favicon</option>
        <option value="ios">iOS</option>
        <option value="android">Android</option>
        <option value="custom">Custom</option>
      </NativeSelect>
    </div>
    <fieldset className="workspace-fieldset">
      <legend>Icon sizes</legend>
      <div className="icon-size-grid" aria-label="Icon size presets">
        {iconSizesByPreset[options.preset].map((size) => (
          <Label key={size} className="workspace-check" htmlFor={`icon-size-${size}`}>
            <Checkbox
              id={`icon-size-${size}`}
              aria-label={`Icon size ${size}px`}
              checked={options.sizes.includes(size)}
              onCheckedChange={(checked) => setOptions((current) => ({
                ...current,
                sizes: checked
                  ? [...new Set([...current.sizes, size])].sort((left, right) => left - right)
                  : current.sizes.filter((value) => value !== size),
              }))}
            />
            <span>{size}px</span>
          </Label>
        ))}
      </div>
    </fieldset>
    <p className="workspace-panel-copy">Each image's icons are in one folder, with a folder per platform inside. A set is saved only when every size worked, so you never get half a set.</p>
  </>
);

export const iconSetTool: MediaTool<IconSetOptions> = {
  initialOptions: { preset: "macos", sizes: iconSizesByPreset.macos },
  Panel: IconSetPanel,
  // Each master makes its own set, so every chosen image is sent in one run.
  buildRequest: ({ preset, sizes }, { paths, outputLocation }) => ({
    command: "generate_icon_set",
    request: { paths, preset, sizes, outputLocation },
  }),
  runLabel: (fileCount) => fileCount === 0 ? "Make icon sets" : fileCount === 1 ? "Make 1 icon set" : `Make ${fileCount} icon sets`,
};
