/**
 * How long a frame stays on screen in the saved GIF. GIF stores delays in
 * hundredths of a second, so a delay rounds to the nearest 10 ms and never to
 * zero, the same rounding the native encoder applies.
 */
export const playedDelayMs = (delayMs: number): number => Math.max(1, Math.floor((delayMs + 5) / 10)) * 10;

/**
 * The size above which a new GIF counts as large: 10 MB, the upload cap of
 * common places a GIF is shared, such as GitHub images and Discord files.
 */
export const LARGE_GIF_BYTES = 10_000_000;

/**
 * About how many bytes the GIF takes: one palette index per canvas pixel per
 * frame, which is what the encoder writes before LZW compression. The canvas is
 * frame 1's size, so other frames' sizes do not change it.
 */
export const estimateGifBytes = (canvas: { width: number; height: number }, frameCount: number): number =>
  canvas.width * canvas.height * frameCount;

export const isLargeGif = (bytes: number): boolean => bytes > LARGE_GIF_BYTES;
