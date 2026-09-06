# Blur Faces

Open `Blur Faces` from the Images tools. In the current build the registry
marks this feature unavailable, so the route ends at `Unavailable in this
build.` and no file chooser is shown. Verify that state and that no file is
selected or processed. The prerequisite for a producing drive is a configured
offline face-blur adapter (`TOOLBOX_FACE_BLUR_PATH`, a bundled
`vision/manifest.json` resource, or `toolbox-face-blur` on `PATH`); in a build
with that resource, select a photo and verify `-blurred.png`, or an explicit
unavailable result with no fake output when the adapter is absent.
