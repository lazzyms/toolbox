# Blur Faces

Open `Blur Faces` (`Runs through the configured offline face adapter.`), select a photo that contains a clearly visible face, and press `Blur Faces`. Expect `Blurred faces` and a `{stem}-blurred.png`. Verify the face regions are blurred, the rest of the image is unchanged, and the source is untouched.

Prerequisites: the `toolbox-vision-adapter` binary and the YuNet model wired through `TOOLBOX_FACE_BLUR_PATH` and `TOOLBOX_FACE_BLUR_MODEL_PATH` (see the Launch section), and a fixture with a detectable face. An image without a face fails with `No face was detected in the image.` and writes nothing; that proves the failure path only. Without the adapter, expect an explicit unavailable result and no output.
