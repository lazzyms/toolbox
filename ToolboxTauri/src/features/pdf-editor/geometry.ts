import type { PdfPage, PdfRect, PreviewSize } from "./contracts";

const clamp = (value: number, min: number, max: number) => Math.min(Math.max(value, min), max);

export const previewToPdfRect = (rect: PdfRect, page: PdfPage, preview: PreviewSize): PdfRect => {
    const scaleX = page.width / preview.width;
    const scaleY = page.height / preview.height;
    const left = clamp(rect.x * scaleX, 0, page.width);
    const top = clamp(rect.y * scaleY, 0, page.height);
    const right = clamp((rect.x + rect.width) * scaleX, 0, page.width);
    const bottom = clamp((rect.y + rect.height) * scaleY, 0, page.height);
    const pageX = page.x ?? 0;
    const pageY = page.y ?? 0;
    return {
        x: pageX + left,
        y: pageY + page.height - bottom,
        width: Math.max(0, right - left),
        height: Math.max(0, bottom - top),
    };
};

export const pdfToPreviewRect = (rect: PdfRect, page: PdfPage, preview: PreviewSize): PdfRect => {
    const pageX = page.x ?? 0;
    const pageY = page.y ?? 0;
    return {
        x: (rect.x - pageX) * preview.width / page.width,
        y: (pageY + page.height - rect.y - rect.height) * preview.height / page.height,
        width: rect.width * preview.width / page.width,
        height: rect.height * preview.height / page.height,
    };
};

export const visiblePageIndices = (pages: PdfPage[], currentPage: number, radius = 2): number[] => {
    if (pages.length === 0) return [];
    const start = Math.max(0, currentPage - radius);
    const end = Math.min(pages.length - 1, currentPage + radius);
    return pages.slice(start, end + 1).map((page) => page.index);
};
