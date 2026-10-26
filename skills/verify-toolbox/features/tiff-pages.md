# TIFF Pages

Open `Open Split and Combine TIFF` from the Images category. There are no options; the run button is `TIFF Pages`.

- One file selected: the tool splits it into `<stem>-page-1.tiff` through `-page-N.tiff`, with the detail `TIFF pages saved`. A single-page TIFF still produces `-page-1.tiff`.
- Two or more files selected: the tool combines every page into one `<first-stem>-combined.tiff`, with the detail `TIFF pages combined`. Combine mode is all or nothing; one unreadable input fails the whole batch.

Only 8-bit Gray, RGB, and RGBA TIFFs are supported; other formats fail with `Unsupported TIFF color type or bit depth: ...`. Verify the source bytes are unchanged.
