import type { PdfPage, PdfRect, PreviewSize } from "./contracts";

const clamp = (value: number, min: number, max: number) => Math.min(Math.max(value, min), max);

export const previewToPdfRect = (rect: PdfRect, page: PdfPage, preview: PreviewSize): PdfRect => {
    const scaleX = page.width / preview.width;
    const scaleY = page.height / preview.height;
    const x = clamp(rect.x * scaleX, 0, page.width);
    const y = clamp(rect.y * scaleY, 0, page.height);
    const right = clamp((rect.x + rect.width) * scaleX, 0, page.width);
    const bottom = clamp((rect.y + rect.height) * scaleY, 0, page.height);
    return { x, y, width: Math.max(0, right - x), height: Math.max(0, bottom - y) };
};

export const pdfToPreviewRect = (rect: PdfRect, page: PdfPage, preview: PreviewSize): PdfRect => ({
    x: rect.x * preview.width / page.width,
    y: rect.y * preview.height / page.height,
    width: rect.width * preview.width / page.width,
    height: rect.height * preview.height / page.height,
});

export const visiblePageIndices = (pages: PdfPage[], currentPage: number, radius = 2): number[] => {
    if (pages.length === 0) return [];
    const start = Math.max(0, currentPage - radius);
    const end = Math.min(pages.length - 1, currentPage + radius);
    return pages.slice(start, end + 1).map((page) => page.index);
};
