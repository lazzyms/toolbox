# Product demo video

The checked-in MP4s are rendered from the release macOS app. The Edit PDF demo
uses one generated fictional PDF; the Remove Password demo selects three
generated password-protected PDFs together, enters their shared demo password,
and verifies three unlocked copies. The source PDFs and outputs live in a
temporary directory.

To render locally from app-window screenshots:

```sh
node scripts/product-demo/render-demo.mjs \
  --home docs/assets/toolbox-demo-home.jpg \
  --edit docs/assets/toolbox-demo-edit-success.jpg \
  --output /tmp/toolbox-product-demo.mp4
```

The renderer refuses to replace an existing video or poster; choose a new
output path for each render. Pass --template
scripts/product-demo/remove-password-composition to render the batch password
removal version from the same two screenshot slots (--home for the selected
batch and --edit for the success results).

`flatten-screenshot.mjs` places the transparent app-window captures on the
Toolbox charcoal background. `prepare-demo.mjs` stages a temporary HyperFrames
project, downloads the Mixkit track, verifies its pinned SHA-256, and creates a
faded 20.2-second music bed.
The raw music file is removed with the temporary project. Do not check in the
source MP3. The checked-in attribution file is also uploaded with each release
video.

The product-demo job in tauri-release.yml runs after publishing a release.
It installs the macOS app archive produced by that same release, captures the
app window before and after both workflows, renders fresh MP4s, and attaches
both videos and posters to the release.
