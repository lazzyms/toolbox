import type { AtomicToolId } from "../../contracts";
import { convertImagesTool, type ConvertImagesOptions } from "./convertImages";
import { gifCreateTool, type GifCreateOptions } from "./gifCreate";
import { gifExtractTool } from "./gifExtract";
import { iconSetTool, type IconSetOptions } from "./iconSet";
import { imageMetadataTool, type ImageMetadataOptions } from "./imageMetadata";
import type { TiffPagesState } from "./TiffPageInspector";
import { tiffPagesTool } from "./tiffPages";
import type { MediaTool } from "./types";

export type { MediaPanelProps, MediaRequest, MediaRunInput, MediaTool } from "./types";

/** Each Media tool's options, keyed by the rail entry that selects the tool. */
export type MediaOptions = {
  "icon-set": IconSetOptions;
  "gif-create": GifCreateOptions;
  "gif-extract": Record<string, never>;
  "tiff-pages": TiffPagesState;
  "image-metadata": ImageMetadataOptions;
  "heic-convert": ConvertImagesOptions;
};

export type MediaToolId = keyof MediaOptions;

export const mediaTools: { [Id in MediaToolId]: MediaTool<MediaOptions[Id]> } = {
  "icon-set": iconSetTool,
  "gif-create": gifCreateTool,
  "gif-extract": gifExtractTool,
  "tiff-pages": tiffPagesTool,
  "image-metadata": imageMetadataTool,
  "heic-convert": convertImagesTool,
};

export const isMediaToolId = (id: AtomicToolId): id is MediaToolId => id in mediaTools;

export const initialMediaOptions = (): MediaOptions => ({
  "icon-set": mediaTools["icon-set"].initialOptions,
  "gif-create": mediaTools["gif-create"].initialOptions,
  "gif-extract": mediaTools["gif-extract"].initialOptions,
  "tiff-pages": mediaTools["tiff-pages"].initialOptions,
  "image-metadata": mediaTools["image-metadata"].initialOptions,
  "heic-convert": mediaTools["heic-convert"].initialOptions,
});
