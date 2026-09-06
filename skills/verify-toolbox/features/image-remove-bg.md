# Remove Background

Open `Cutout` from the Images tools. In the current build the registry marks
this feature unavailable, so the route ends at `Unavailable in this build.` and
no file chooser is shown. Verify that state and that no file is selected or
processed. The prerequisite for a producing drive is a configured offline
background-removal adapter (`TOOLBOX_BACKGROUND_REMOVAL_PATH`, a bundled
`vision/manifest.json` resource, or `toolbox-background-removal` on `PATH`);
in a build with that resource, select a photo and verify a transparent
`-cutout.png`, or an explicit unavailable result with no identity copy when the
adapter is absent.
