# Offline vision engines

The Tauri vision tools never upload user files. Each feature calls a local adapter and reports an actionable unsupported error when that adapter is not installed.

| Feature | Adapter variable | Default executable | Output |
| --- | --- | --- | --- |
| PDF OCR | `TOOLBOX_TESSERACT_PATH` | `tesseract` | `-ocr-text.txt` |
| Face blur | `TOOLBOX_FACE_BLUR_PATH` | `toolbox-face-blur` | `-blurred.png` |
| Background removal | `TOOLBOX_BACKGROUND_REMOVAL_PATH` | `toolbox-background-removal` | `-cutout.png` |

Adapters receive the input path followed by the output path, except OCR, which receives the input path and `stdout`. A release bundle must ship signed adapters and their model assets for each supported platform. The application does not silently fall back to a network service or to an identity copy.

The same command and result contracts are used on macOS and Windows. Availability is discovered at runtime so an installation without a bundled model remains usable and explains exactly which capability is unavailable.
