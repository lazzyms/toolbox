# Blur Faces

Open `Open Blur Faces` from the Images category. The tool is registered as implemented, so the dropzone and the `Blur Faces` run button appear along with the text "Runs through the configured offline face adapter."

Select a photo with a face and click `Blur Faces`. The adapter resolves from `TOOLBOX_FACE_BLUR_PATH`, the bundled `vision/manifest.json`, or `toolbox-face-blur` on `PATH`, and it also needs a model (`TOOLBOX_FACE_BLUR_MODEL_PATH` or the bundled `faceBlurModel`). With both present, expect `<name>-blurred.png` (always PNG) and the detail `Blurred faces`. Without them, expect `1 of 1 files failed` with an unavailable message and no `-blurred.png`. A dev checkout has no `vision/manifest.json`, so the unavailable branch is the normal dev result.
