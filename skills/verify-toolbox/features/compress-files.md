# Compress Files

The Compress Files workspace saves a smaller copy of each PDF or image.

## Sub-features

- `compress` writes `-compressed` copies at Low, Medium, or High strength, with optional EXIF removal.

## How to get to it (user POV)

- Choose `Compress Files` in the home rail, or `Open Compress` in the Tool library.

## Driving it with CUA desktop control

Preconditions:

- A JPEG and a PDF fixture copy are in `scratch/`.

- **Compress.** Pick both files. Choose `Medium Good for sharing and email` and leave `Strip EXIF` as it is. Choose `Compress 2 files`. Expect `Done. 2 compressed`, a saving line such as `Saved 25 KB, 52% smaller`, `<image>-compressed.jpg`, and `<pdf>-compressed.pdf`. The image row reports its quality, such as `Re-saved at quality 50.`

## Gotchas

- The saving line counts only compressed files. Skipped and failed files keep their size and get no copy.
- The strength radios read their label and hint together, such as `Low Least saving, detail stays` and `High Smallest, photos look softer`.
