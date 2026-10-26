# Crop Images

Open `Open Crop Images` from the Images category. The `Crop mode` select offers `Rectangle` (default) and `Aspect ratio`. Aspect ratio mode adds a `Crop anchor` select (Center, Top, Bottom, Left, Right). The run button is `Crop`.

The UI has no rectangle or ratio inputs in crop mode. Rectangle mode always crops the top-left 256x256, and aspect mode always crops a 1:1 square at the chosen anchor. Use an image of at least 256x256 and expect `<name>-cropped.<ext>` at exactly 256x256. A smaller image fails with `Crop rectangle must fit inside the image.` A second run writes `-cropped-1`.
