import type { AtomicToolId } from "../../contracts";
import type { ConversionTool } from "./tool";
import { compare } from "./tools/compare";
import { extractPages } from "./tools/extractPages";
import { imagesToPdf } from "./tools/imagesToPdf";
import { merge } from "./tools/merge";
import { ocr } from "./tools/ocr";
import { pdfToImages } from "./tools/pdfToImages";
import { redact } from "./tools/redact";
import { repair } from "./tools/repair";
import { rotate } from "./tools/rotate";
import { split } from "./tools/split";

const conversionTools: Partial<Record<AtomicToolId, ConversionTool>> = {
  "images-to-pdf": imagesToPdf,
  "pdf-to-images": pdfToImages,
  "pdf-ocr": ocr,
  "pdf-merge": merge,
  "pdf-split": split,
  "pdf-extract-pages": extractPages,
  "pdf-rotate": rotate,
  "pdf-repair": repair,
  "pdf-compare": compare,
  "pdf-redact": redact,
};

export const conversionToolFor = (id: AtomicToolId) => {
  const tool = conversionTools[id];
  if (!tool) throw new Error(`${id} is not a PDF conversion tool.`);
  return tool;
};

export { ConversionPanel } from "./ConversionPanel";
