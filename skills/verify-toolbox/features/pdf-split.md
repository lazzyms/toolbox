# Split PDF

Open `Open Split PDF` from the PDF category. The `Split mode` select offers `Every page` (default), `Ranges` (adds `Page ranges`, for example `1-3, 4-8, 9-`), and `Fixed-size chunks` (adds `Pages per file`, default 2). The run button is `Split`. Only the first selected file is split. qpdf is required.

- Every page: expect `<stem>-split-page-1.pdf` through `-page-N.pdf`.
- Ranges: expect `<stem>-split-0.pdf`, `-split-1.pdf`, and so on. Numbering starts at zero.
- Chunks: expect qpdf-numbered `<stem>-split-NN.pdf` files.

The result lists every output path, and the source is unchanged. Clear stale `-split-*` files from earlier runs first, because outputs are found by filename prefix.
