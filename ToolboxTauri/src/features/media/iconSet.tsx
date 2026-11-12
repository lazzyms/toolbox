import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ImagePreview } from "../../contracts";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { MediaPanelProps, MediaTool } from "./types";

/** How a non-square master becomes square: centred on a transparent square, or its centred square kept. */
export type SquareRule = "pad" | "crop";

export interface IconSetOptions {
  preset: string;
  sizes: number[];
  squareRule: SquareRule;
}

const iconSizesByPreset: Record<string, number[]> = {
  macos: [16, 32, 128, 256, 512, 1024],
  favicon: [16, 32, 48, 64, 128, 256],
  ios: [20, 29, 40, 60, 76, 83, 1024],
  android: [48, 72, 96, 144, 192, 512],
  custom: [16, 32, 64, 128, 256, 512],
};

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

const appliesToAll = "Applies to every non-square image in this run. Images are never stretched.";

/** What the rule does to this master, for the preview's name, the rule's hint and the upscale warning. */
const describeSquare = (name: string, rule: SquareRule, size: { width: number; height: number } | null, sizes: number[]) => {
  if (!size) {
    return {
      label: `Preview of ${name}`,
      hint: rule === "pad"
        ? `The whole image is centred on a transparent square of its longer side. ${appliesToAll}`
        : `The largest centred square is kept. ${appliesToAll}`,
      warning: null,
    };
  }
  const { width, height } = size;
  if (width === height) {
    return { label: `${name}, already square`, hint: `${name} is already square, so it is used as it is. The rule applies to every non-square image in this run. Images are never stretched.`, warning: null };
  }
  if (rule === "pad") {
    const side = Math.max(width, height);
    return { label: `${name} padded to a transparent ${side}-pixel square`, hint: `The whole image is centred on a transparent square of ${side} pixels. ${appliesToAll}`, warning: null };
  }
  const side = Math.min(width, height);
  const edges = width > height ? "left and right" : "top and bottom";
  const upscaled = sizes.filter((value) => value > side);
  return {
    label: `${name} cropped to its centred ${side}-pixel square`,
    hint: `The centred ${side}-pixel square is kept. The ${edges} edges are cut off. ${appliesToAll}`,
    warning: upscaled.length === 0
      ? null
      : `The crop is ${side} pixels, so ${upscaled.length === 1 ? `the ${upscaled[0]} size` : `sizes above ${side}`} will be upscaled.`,
  };
};

/** The focused master with the run's square rule applied, and the control that picks the rule. */
const SquarePreview = ({ utility, options, setOptions, files, selectedFileIndex }: MediaPanelProps<IconSetOptions>) => {
  const focused = files[selectedFileIndex] ?? files[0] ?? null;
  const supportsPreview = utility.capability.supportsPreview;
  const [preview, setPreview] = useState<{ path: string; image: ImagePreview | null } | null>(null);

  useEffect(() => {
    if (!focused || !supportsPreview) return;
    let current = true;
    invoke<ImagePreview>("inspect_image_preview", { request: { path: focused } })
      .then((image) => { if (current) setPreview({ path: focused, image }); })
      .catch(() => { if (current) setPreview({ path: focused, image: null }); });
    return () => {
      current = false;
    };
  }, [focused, supportsPreview]);

  const image = focused && preview?.path === focused ? preview.image : null;
  const name = focused ? fileName(focused) : null;
  const square = describeSquare(name ?? "", options.squareRule, image, options.sizes);

  return (
    <Card className="icon-square py-0" role="region" aria-label="Preview">
      <CardContent className="icon-square-content py-4">
        <p className="icon-square-heading">
          <span className="workspace-panel-label">Preview</span>
          {name && <span className="workspace-panel-copy">{name}</span>}
        </p>
        {name && supportsPreview && (
          <div className="icon-square-preview" role="img" aria-label={square.label} data-square-rule={options.squareRule}>
            {image && <img src={image.dataUrl} alt="" />}
          </div>
        )}
        <div className="icon-square-rule">
          <p className="workspace-panel-label" id="icon-square-rule-label">Non-square images</p>
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            aria-labelledby="icon-square-rule-label"
            aria-describedby="icon-square-rule-hint"
            value={options.squareRule}
            onValueChange={(value) => {
              if (value) setOptions((current) => ({ ...current, squareRule: value as SquareRule }));
            }}
          >
            <ToggleGroupItem value="pad">Pad with transparency</ToggleGroupItem>
            <ToggleGroupItem value="crop">Crop to centre</ToggleGroupItem>
          </ToggleGroup>
          <p className="workspace-panel-copy" id="icon-square-rule-hint">{square.hint}</p>
          {name && square.warning && <p className="workspace-note" role="status">{square.warning}</p>}
        </div>
      </CardContent>
    </Card>
  );
};

const IconSetPanel = (props: MediaPanelProps<IconSetOptions>) => {
  const { options, setOptions } = props;
  return (
  <>
    <SquarePreview {...props} />
    <div className="workspace-field">
      <Label htmlFor="icon-preset">Icon preset</Label>
      <NativeSelect
        id="icon-preset"
        aria-label="Icon preset"
        value={options.preset}
        onChange={(event) => {
          const preset = event.target.value;
          setOptions((current) => ({ ...current, preset, sizes: iconSizesByPreset[preset] ?? iconSizesByPreset.custom }));
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
};

export const iconSetTool: MediaTool<IconSetOptions> = {
  initialOptions: { preset: "macos", sizes: iconSizesByPreset.macos, squareRule: "pad" },
  Panel: IconSetPanel,
  // Each master makes its own set, so every chosen image is sent in one run.
  buildRequest: ({ preset, sizes, squareRule }, { paths, outputLocation }) => ({
    command: "generate_icon_set",
    request: { paths, preset, sizes, squareRule, outputLocation },
  }),
  runLabel: (_options, files) => files.length === 0 ? "Make icon sets" : files.length === 1 ? "Make 1 icon set" : `Make ${files.length} icon sets`,
};
