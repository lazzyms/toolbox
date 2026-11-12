import type { ConvertImagesRequest, TiffCompression } from "../contracts";

// The formats Convert Image Format writes, in the board's order. The native
// command accepts each of these, and only TIFF takes a compression.
export const convertTargets: { format: ConvertImagesRequest["format"]; name: string; hint: string }[] = [
  { format: "jpg", name: "JPEG", hint: "Transparent areas become white, because JPEG has no transparency." },
  { format: "png", name: "PNG", hint: "Lossless. Transparency is kept." },
  { format: "webp", name: "WebP", hint: "Follows each source. PNG and TIFF stay lossless. JPEG and HEIC are lossy." },
  { format: "heic", name: "HEIC", hint: "Saved as 8-bit. Transparency is kept." },
  { format: "tiff", name: "TIFF", hint: "Lossless. Transparency is kept." },
];

export const tiffCompressions: { value: TiffCompression; name: string }[] = [
  { value: "lzw", name: "LZW" },
  { value: "none", name: "None" },
  { value: "deflate", name: "Deflate" },
];

export const defaultTiffCompression: TiffCompression = "lzw";

// The compression travels only with a TIFF target.
export const convertImagesRequest = (
  paths: string[],
  format: ConvertImagesRequest["format"],
  tiffCompression: TiffCompression,
  outputLocation: ConvertImagesRequest["outputLocation"],
): ConvertImagesRequest => format === "tiff"
  ? { paths, format, tiffCompression, outputLocation }
  : { paths, format, outputLocation };
