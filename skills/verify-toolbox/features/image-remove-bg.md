# Remove Background

Open `Remove Background` (`Runs through the configured offline background adapter.`), select a photo, and press `Cutout`. Expect `Background removed` and a `{stem}-cutout.png`. Verify the output has an alpha channel with a transparent background and the source is unchanged.

Prerequisites: the `toolbox-vision-adapter` binary and the U2-Net model wired through `TOOLBOX_BACKGROUND_REMOVAL_PATH` and `TOOLBOX_BACKGROUND_REMOVAL_MODEL_PATH` (see the Launch section). Without them, expect an explicit unavailable result and no identity copy.
