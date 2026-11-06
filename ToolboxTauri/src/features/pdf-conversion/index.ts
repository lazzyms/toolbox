import type { AtomicToolId } from "../../contracts";
import type { ConversionTool } from "./tool";
import { compare } from "./tools/compare";
import { convertToPdf } from "./tools/convertToPdf";
import { extractPages } from "./tools/extractPages";
import { imagesToPdf } from "./tools/imagesToPdf";
import { merge } from "./tools/merge";
import { ocr } from "./tools/ocr";
import { pdfImageExtract } from "./tools/pdfImageExtract";
import { pdfToImages } from "./tools/pdfToImages";
import { pdfToText } from "./tools/pdfToText";
import { redact } from "./tools/redact";
import { repair } from "./tools/repair";
import { rotate } from "./tools/rotate";
import { split } from "./tools/split";

const conversionTools: Partial<Record<AtomicToolId, ConversionTool>> = {
  "images-to-pdf": imagesToPdf,
  "pdf-to-images": pdfToImages,
  "pdf-to-text": pdfToText,
  "pdf-image-extract": pdfImageExtract,
  "pdf-ocr": ocr,
  "pdf-merge": merge,
  "pdf-split": split,
  "pdf-extract-pages": extractPages,
  "pdf-rotate": rotate,
  "pdf-repair": repair,
  "pdf-compare": compare,
  "pdf-redact": redact,
  "convert-to-pdf": convertToPdf,
};

export const conversionToolFor = (id: AtomicToolId) => {
  const tool = conversionTools[id];
  if (!tool) throw new Error(`${id} is not a PDF conversion tool.`);
  return tool;
};

export { ConversionPanel } from "./ConversionPanel";
