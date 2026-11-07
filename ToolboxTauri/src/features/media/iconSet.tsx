import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronDown, ChevronRight, X } from "lucide-react";
import type { ImagePreview } from "../../contracts";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { MediaPanelProps, MediaTool } from "./types";

/** How a non-square master becomes square: centred on a transparent square, or its centred square kept. */
export type SquareRule = "pad" | "crop";

export type IconPreset = "macos" | "ios" | "android" | "favicon" | "custom";

/** A file that bundles a platform's sizes: an Xcode asset catalog folder, a macOS ICNS or a favicon ICO. */
export type IconContainer = "appiconset" | "icns" | "ico";

/**
 * Whether a platform is ticked, the sizes ticked for it and the containers ticked for it.
 * Its sizes and containers are kept while it is unticked.
 */
export interface IconPlatformChoice {
  on: boolean;
  sizes: number[];
  containers: IconContainer[];
}

export interface IconSetOptions {
  platforms: Record<IconPreset, IconPlatformChoice>;
  squareRule: SquareRule;
}

interface PlatformInfo {
  preset: IconPreset;
  /** The row's name, which is also its checkbox's accessible name. */
  label: string;
  /** The name the contact sheet uses for this platform's sizes. */
  sheetName: string;
  /** The sizes the platform offers, as the design lists them. Custom sizes are typed in instead. */
  offered: number[];
  /** The containers the platform offers, in the design's order. */
  containers: IconContainer[];
  /** The name the platform's notes use for its sizes. */
  sizesName: string;
  hint?: string;
}

const platforms: PlatformInfo[] = [
  { preset: "macos", label: "macOS app", sheetName: "macOS", sizesName: "macOS", offered: [16, 32, 64, 128, 256, 512, 1024], containers: ["appiconset", "icns"] },
  {
    preset: "ios",
    label: "iPhone and iPad",
    sheetName: "iPhone and iPad",
    sizesName: "iPhone and iPad",
    offered: [20, 29, 40, 58, 60, 76, 80, 87, 120, 152, 167, 180, 1024],
    containers: ["appiconset"],
    hint: "iPhone and iPad icons can't be transparent. Transparent areas are filled with white.",
  },
  { preset: "android", label: "Android", sheetName: "Android", sizesName: "Android", offered: [48, 72, 96, 144, 192, 512], containers: [], hint: "Launcher sizes from mdpi to xxxhdpi, plus the 512 store icon." },
  { preset: "favicon", label: "Website", sheetName: "Website", sizesName: "Website", offered: [16, 32, 48, 64, 128, 256], containers: ["ico"] },
  { preset: "custom", label: "Custom sizes", sheetName: "custom", sizesName: "custom", offered: [], containers: [] },
];

const platformInfo = (preset: IconPreset) => platforms.find((platform) => platform.preset === preset)!;

export const customSizeRange = { min: 1, max: 4096 } as const;

const ascending = (sizes: Iterable<number>) => [...new Set(sizes)].sort((left, right) => left - right);

const sizeCount = (count: number) => (count === 1 ? "1 size" : `${count} sizes`);

/** The platform's sizes that aren't ticked. */
const missingSizes = (platform: PlatformInfo, sizes: number[]) => platform.offered.filter((size) => !sizes.includes(size));

/** The asset catalog and the ICNS need every size of their preset. The favicon.ico holds whichever sizes are ticked. */
const containerAvailable = (container: IconContainer, platform: PlatformInfo, sizes: number[]) =>
  container === "ico" || missingSizes(platform, sizes).length === 0;

/** The containers that will be made: the ticked ones whose sizes allow them, in the platform's order. */
const madeContainers = (platform: PlatformInfo, choice: IconPlatformChoice) =>
  platform.containers.filter((container) => choice.containers.includes(container) && containerAvailable(container, platform, choice.sizes));

const containerSummary: Record<IconContainer, string> = { icns: "ICNS", appiconset: ".appiconset", ico: "favicon.ico" };

/** The summary beside a platform's name. */
const platformSummary = (platform: PlatformInfo, choice: IconPlatformChoice) => {
  if (choice.sizes.length === 0) return platform.preset === "custom" ? "None yet" : "No sizes ticked";
  const made = madeContainers(platform, choice);
  // The design lists the ICNS before the asset catalog.
  const names = (["icns", "appiconset", "ico"] as const).filter((container) => made.includes(container)).map((container) => containerSummary[container]);
  if (names.length > 0) return `${sizeCount(choice.sizes.length)}, ${names.join(", ")}`;
  return platform.containers.some((container) => container !== "ico") ? `${sizeCount(choice.sizes.length)}, PNGs only` : sizeCount(choice.sizes.length);
};

/** The ticked platforms with the sizes and containers made for each, in the design's order. */
const tickedPlatforms = (options: IconSetOptions) =>
  platforms.filter((platform) => options.platforms[platform.preset].on).map((platform) => {
    const choice = options.platforms[platform.preset];
    return { ...platform, sizes: choice.sizes, containers: madeContainers(platform, choice) };
  });

/** Why the run can't start yet, or null when it can. */
const blockedReason = (options: IconSetOptions) => {
  const ticked = tickedPlatforms(options);
  if (ticked.length === 0) return "Tick at least one platform to make icons for.";
  const empty = ticked.find((platform) => platform.sizes.length === 0);
  return empty ? `Tick at least one size for ${empty.label}, or untick it.` : null;
};

/** A typed custom size, or why it can't be added. */
export const parseCustomSize = (value: string, current: number[]): { size: number } | { error: string } => {
  const trimmed = value.trim();
  if (!/^\d+$/.test(trimmed)) return { error: `Type a whole number of pixels from ${customSizeRange.min} to ${customSizeRange.max}.` };
  const size = Number(trimmed);
  if (size < customSizeRange.min || size > customSizeRange.max) return { error: `${size} pixels is outside the range. Use ${customSizeRange.min} to ${customSizeRange.max} pixels.` };
  if (current.includes(size)) return { error: `${size} pixels is already in the list.` };
  return { size };
};

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

/** A file's name without its extension, which names the ICNS. */
const fileStem = (path: string) => fileName(path).replace(/\.[^.]*$/, "");

/** The container's checkbox label. The ICNS is named after the image, as each image makes its own set. */
const containerLabel = (container: IconContainer, firstFile: string | null) => {
  if (container === "appiconset") return "AppIcon.appiconset, with Contents.json";
  if (container === "ico") return "favicon.ico, with the ticked sizes up to 256";
  return firstFile ? `${fileStem(firstFile)}.icns` : "ICNS, named after each image";
};

/** The note under a platform whose asset catalog or ICNS is off because a size is unticked. */
const containersOffNote = (platform: PlatformInfo, missing: number[]) => {
  const needy = platform.containers.filter((container) => container !== "ico");
  const subject = needy.length > 1 ? "AppIcon.appiconset and the ICNS need" : "AppIcon.appiconset needs";
  const them = needy.length > 1 ? "they're" : "it's";
  const back = needy.length > 1 ? "them" : "it";
  return `${subject} every ${platform.sizesName} size, so ${them} off. Tick ${listSizes(missing)} to turn ${back} back on.`;
};

const appliesToAll = "Applies to every non-square image in this run. Images are never stretched.";

const listSizes = (sizes: number[]) => sizes.length === 1 ? `${sizes[0]}` : `${sizes.slice(0, -1).join(", ")} and ${sizes[sizes.length - 1]}`;

/** The side of the square every size is made from: the longer side when padded, the shorter when cropped. */
const squareSide = (rule: SquareRule, size: { width: number; height: number }) =>
  rule === "pad" ? Math.max(size.width, size.height) : Math.min(size.width, size.height);

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
  const side = squareSide(rule, size);
  const upscaled = sizes.filter((value) => value > side);
  const which = upscaled.length === 1 ? `the ${upscaled[0]} size` : `sizes above ${side}`;
  const warning = (subject: string) => (upscaled.length === 0 ? null : `${subject} ${side} pixels, so ${which} will be upscaled.`);
  if (width === height) {
    return {
      label: `${name}, already square`,
      hint: `${name} is already square, so it is used as it is. The rule applies to every non-square image in this run. Images are never stretched.`,
      warning: warning(`${name} is`),
    };
  }
  if (rule === "pad") {
    return {
      label: `${name} padded to a transparent ${side}-pixel square`,
      hint: `The whole image is centred on a transparent square of ${side} pixels. ${appliesToAll}`,
      warning: warning("The padded square is"),
    };
  }
  const edges = width > height ? "left and right" : "top and bottom";
  return {
    label: `${name} cropped to its centred ${side}-pixel square`,
    hint: `The centred ${side}-pixel square is kept. The ${edges} edges are cut off. ${appliesToAll}`,
    warning: warning("The crop is"),
  };
};

/** The largest size the contact sheet shows at actual pixels; larger sizes are shown at 96. */
const actualPixelsUpTo = 64;
const scaledTile = 96;

/** The focused master at each of one platform's ticked sizes, with upscaled sizes marked. */
const ContactSheet = ({ name, platform, sizes, image, rule, side }: {
  name: string;
  platform: PlatformInfo;
  sizes: number[];
  image: ImagePreview | null;
  rule: SquareRule;
  side: number | null;
}) => (
  <div className="icon-contact">
    <p className="icon-square-heading">
      <span className="workspace-panel-label">Contact sheet, {platform.sheetName} sizes</span>
      <span className="workspace-panel-copy">Actual pixels up to {actualPixelsUpTo}. Larger sizes are shown at {scaledTile}.</span>
    </p>
    <div className="icon-contact-sheet" role="img" aria-label={`${name} at ${listSizes(sizes)} pixels`} data-square-rule={rule}>
      {sizes.map((size) => {
        const tile = size > actualPixelsUpTo ? scaledTile : size;
        const upscaled = side !== null && size > side;
        return (
          <div key={size} className="icon-contact-tile" data-size={size}>
            <div className="icon-contact-image" style={{ width: tile, height: tile }}>
              {image && <img src={image.dataUrl} alt="" />}
            </div>
            <span className={upscaled ? "icon-contact-size icon-contact-upscaled" : "icon-contact-size"}>{upscaled ? `${size} ↑` : size}</span>
          </div>
        );
      })}
    </div>
  </div>
);

/** The focused master with the run's square rule applied, the control that picks the rule, and the contact sheet. */
const SquarePreview = ({ utility, options, setOptions, files, selectedFileIndex, sheetPreset }: MediaPanelProps<IconSetOptions> & { sheetPreset: IconPreset | null }) => {
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
  const allSizes = ascending(tickedPlatforms(options).flatMap((platform) => platform.sizes));
  const square = describeSquare(name ?? "", options.squareRule, image, allSizes);
  const sheetSizes = sheetPreset ? options.platforms[sheetPreset].sizes : [];

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
          {name && square.warning && <p className="workspace-note workspace-note-warning" role="status">{square.warning}</p>}
        </div>
        {name && supportsPreview && sheetPreset && sheetSizes.length > 0 && (
          <ContactSheet
            name={name}
            platform={platformInfo(sheetPreset)}
            sizes={sheetSizes}
            image={image}
            rule={options.squareRule}
            side={image ? squareSide(options.squareRule, image) : null}
          />
        )}
      </CardContent>
    </Card>
  );
};

/** The typed sizes of the Custom row: a field to add one, and a chip to remove each. */
const CustomSizes = ({ sizes, setSizes }: { sizes: number[]; setSizes: (sizes: number[]) => void }) => {
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const add = () => {
    const parsed = parseCustomSize(draft, sizes);
    if ("error" in parsed) {
      setError(parsed.error);
      return;
    }
    setSizes(ascending([...sizes, parsed.size]));
    setDraft("");
    setError(null);
  };
  return (
    <div className="icon-custom-sizes" role="group" aria-label="Sizes, pixels">
      <div className="icon-custom-add">
        <Input
          className="icon-custom-input"
          aria-label="Add a size in pixels"
          placeholder={`${customSizeRange.min} to ${customSizeRange.max}`}
          inputMode="numeric"
          value={draft}
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? "icon-custom-error" : "icon-custom-hint"}
          onChange={(event) => {
            setDraft(event.target.value);
            setError(null);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              add();
            }
          }}
        />
        <Button type="button" variant="outline" size="sm" onClick={add}>Add size</Button>
      </div>
      {error && <p className="workspace-note workspace-note-error" id="icon-custom-error" role="alert">{error}</p>}
      {sizes.length > 0 && (
        <ul className="icon-custom-chips" aria-label="Custom sizes added">
          {sizes.map((size) => (
            <li key={size}>
              <Badge variant="outline" className="icon-custom-chip">
                {size}
                <Button type="button" variant="ghost" size="icon-xs" aria-label={`Remove ${size}`} onClick={() => setSizes(sizes.filter((value) => value !== size))}>
                  <X aria-hidden="true" />
                </Button>
              </Badge>
            </li>
          ))}
        </ul>
      )}
      <p className="workspace-panel-copy" id="icon-custom-hint">Saved as custom-{sizes[0] ?? 300}.png and so on. Square, {customSizeRange.min} to {customSizeRange.max} pixels.</p>
    </div>
  );
};

const IconSetPanel = (props: MediaPanelProps<IconSetOptions>) => {
  const { options, setOptions } = props;
  const [expanded, setExpanded] = useState<IconPreset | null>("macos");
  const setPlatform = (preset: IconPreset, update: (choice: IconPlatformChoice) => IconPlatformChoice) =>
    setOptions((current) => ({ ...current, platforms: { ...current.platforms, [preset]: update(current.platforms[preset]) } }));
  // The contact sheet follows the open row while it is ticked, and otherwise the first ticked platform with sizes.
  const ticked = tickedPlatforms(options).filter((platform) => platform.sizes.length > 0);
  const sheetPreset = ticked.find((platform) => platform.preset === expanded)?.preset ?? ticked[0]?.preset ?? null;
  const blocked = blockedReason(options);
  const firstFile = props.files[0] ?? null;

  return (
    <>
      <SquarePreview {...props} sheetPreset={sheetPreset} />
      <fieldset className="workspace-fieldset icon-platforms">
        <legend>Make icons for</legend>
        {platforms.map((platform) => {
          const choice = options.platforms[platform.preset];
          const open = expanded === platform.preset;
          const groupId = `icon-sizes-${platform.preset}`;
          return (
            <div key={platform.preset} className="icon-platform">
              <div className="icon-platform-row">
                <Checkbox
                  id={`icon-platform-${platform.preset}`}
                  aria-label={platform.label}
                  checked={choice.on}
                  onCheckedChange={(checked) => setPlatform(platform.preset, (current) => ({ ...current, on: checked === true }))}
                />
                <Label htmlFor={`icon-platform-${platform.preset}`} className="icon-platform-name">{platform.label}</Label>
                <span className="workspace-panel-copy icon-platform-summary">{platformSummary(platform, choice)}</span>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xs"
                  aria-expanded={open}
                  aria-controls={open ? groupId : undefined}
                  aria-label={`Sizes for ${platform.label}`}
                  onClick={() => setExpanded(open ? null : platform.preset)}
                >
                  {open ? <ChevronDown aria-hidden="true" /> : <ChevronRight aria-hidden="true" />}
                </Button>
              </div>
              {open && platform.preset === "custom" && (
                <div id={groupId} className="icon-platform-sizes">
                  <CustomSizes sizes={choice.sizes} setSizes={(sizes) => setPlatform("custom", (current) => ({ ...current, sizes }))} />
                </div>
              )}
              {open && platform.preset !== "custom" && (
                <div id={groupId} className="icon-platform-sizes">
                  <div className="icon-size-grid" role="group" aria-label="Sizes, pixels">
                    {platform.offered.map((size) => (
                      <Label key={size} className="workspace-check" htmlFor={`icon-size-${platform.preset}-${size}`}>
                        <Checkbox
                          id={`icon-size-${platform.preset}-${size}`}
                          checked={choice.sizes.includes(size)}
                          onCheckedChange={(checked) => setPlatform(platform.preset, (current) => ({
                            ...current,
                            sizes: checked === true ? ascending([...current.sizes, size]) : current.sizes.filter((value) => value !== size),
                          }))}
                        />
                        <span>{size}</span>
                      </Label>
                    ))}
                  </div>
                  {platform.containers.length === 0 && platform.hint && <p className="workspace-panel-copy">{platform.hint}</p>}
                  {platform.containers.length > 0 && (
                    <div className="icon-platform-containers" role="group" aria-label={`${platform.label} containers`}>
                      {platform.containers.map((container) => {
                        const available = containerAvailable(container, platform, choice.sizes);
                        const id = `icon-container-${platform.preset}-${container}`;
                        // The label's own disabled style follows its group, so an unavailable container reads as off.
                        return (
                          <div key={container} className="group" data-disabled={!available}>
                            <Label className="workspace-check" htmlFor={id}>
                              <Checkbox
                                id={id}
                                checked={available && choice.containers.includes(container)}
                                disabled={!available}
                                onCheckedChange={(checked) => setPlatform(platform.preset, (current) => ({
                                  ...current,
                                  containers: checked === true ? [...new Set([...current.containers, container])] : current.containers.filter((value) => value !== container),
                                }))}
                              />
                              <span>{containerLabel(container, firstFile)}</span>
                            </Label>
                          </div>
                        );
                      })}
                      {(() => {
                        const missing = missingSizes(platform, choice.sizes);
                        if (missing.length === 0 || choice.sizes.length === 0 || platform.containers.every((container) => container === "ico")) return null;
                        return (
                          <div className="workspace-note icon-platform-note">
                            <p>{containersOffNote(platform, missing)}</p>
                            <Button
                              type="button"
                              variant="outline"
                              size="sm"
                              onClick={() => setPlatform(platform.preset, (current) => ({ ...current, sizes: [...platform.offered] }))}
                            >
                              Tick all {platform.sizesName} sizes
                            </Button>
                          </div>
                        );
                      })()}
                      {platform.hint && <p className="workspace-panel-copy">{platform.hint}</p>}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}
        {blocked && <p className="workspace-note workspace-note-error" role="alert">{blocked}</p>}
      </fieldset>
      <p className="workspace-panel-copy">Each image's icons are in one folder, with a folder per platform inside. A set is saved only when every size worked, so you never get half a set.</p>
    </>
  );
};

const allSizes = (preset: IconPreset, on: boolean): IconPlatformChoice => ({ on, sizes: [...platformInfo(preset).offered], containers: [...platformInfo(preset).containers] });

export const iconSetTool: MediaTool<IconSetOptions> = {
  initialOptions: {
    platforms: {
      macos: allSizes("macos", true),
      ios: allSizes("ios", true),
      android: allSizes("android", false),
      favicon: allSizes("favicon", true),
      custom: { on: false, sizes: [], containers: [] },
    },
    squareRule: "pad",
  },
  Panel: IconSetPanel,
  canRun: (options) => blockedReason(options) === null,
  // Each master makes its own set, so every chosen image is sent in one run.
  buildRequest: (options, { paths, outputLocation }) => ({
    command: "generate_icon_set",
    request: {
      paths,
      platforms: tickedPlatforms(options).map(({ preset, sizes, containers }) => ({ preset, sizes, containers })),
      squareRule: options.squareRule,
      outputLocation,
    },
  }),
  runLabel: (fileCount) => fileCount === 0 ? "Make icon sets" : fileCount === 1 ? "Make 1 icon set" : `Make ${fileCount} icon sets`,
};
