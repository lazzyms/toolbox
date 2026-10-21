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

/** A box on a rendered page, as fractions of the displayed width and height measured from the top-left corner. */
export interface DisplayedBox {
    left: number;
    top: number;
    right: number;
    bottom: number;
}

/** Maps a box drawn on a rendered page preview (which shows the crop box after /Rotate) back into unrotated PDF points. */
export const displayedBoxToPdfRect = (box: DisplayedBox, page: PdfPage): PdfRect => {
    const [left, bottom, right, top] = page.pageBox ?? [page.x ?? 0, page.y ?? 0, (page.x ?? 0) + page.width, (page.y ?? 0) + page.height];
    const width = right - left;
    const height = top - bottom;
    const toPdf = (u: number, v: number): [number, number] => {
        switch (((page.rotation ?? 0) % 360 + 360) % 360) {
            case 90: return [left + v * width, bottom + u * height];
            case 180: return [right - u * width, bottom + v * height];
            case 270: return [right - v * width, top - u * height];
            default: return [left + u * width, top - v * height];
        }
    };
    const [x0, y0] = toPdf(clamp(box.left, 0, 1), clamp(box.top, 0, 1));
    const [x1, y1] = toPdf(clamp(box.right, 0, 1), clamp(box.bottom, 0, 1));
    return { x: Math.min(x0, x1), y: Math.min(y0, y1), width: Math.abs(x1 - x0), height: Math.abs(y1 - y0) };
};
