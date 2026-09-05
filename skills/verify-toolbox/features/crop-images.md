# Crop Images

Choose `All tools` → `Images` → `Crop Images`, select an image, and run the crop.

## Driving it with Tauri desktop UI

The default `Rectangle` mode sends a fixed 256 × 256 rectangle from the current UI; there are no numeric x/y/width/height fields. `Aspect ratio` mode uses the configured width/height values and an optional anchor. Use an image at least 256 × 256 for the default rectangle, then verify the output dimensions, collision-safe `-cropped` name, and unchanged source bytes.
