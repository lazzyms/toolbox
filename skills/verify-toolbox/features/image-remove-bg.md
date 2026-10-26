# Remove Background

Open `Open Remove Background` from the Images category. The tool is registered as implemented, so the dropzone and the `Cutout` run button appear along with the text "Runs through the configured offline background adapter."

Select a photo and click `Cutout`. The engine resolves from `TOOLBOX_BACKGROUND_REMOVAL_PATH`, the bundled `vision/manifest.json`, or `toolbox-background-removal` on `PATH`, and it also needs a model (`TOOLBOX_BACKGROUND_REMOVAL_MODEL_PATH` or the bundled `backgroundRemovalModel`). With both present, expect `<name>-cutout.png` with a transparent subject mask and the detail `Background removed`. Without them, expect `1 of 1 files failed` with an unavailable message such as `Resource backgroundRemoval is unavailable.` and no output or identity copy. A dev checkout has no `vision/manifest.json`, so the unavailable branch is the normal dev result.
