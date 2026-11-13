import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronLeft, ChevronRight, LoaderCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";

/** One drawn page, as preview_formatted_pdf returns it. */
export type PreviewPage = { dataUrl: string; width: number; height: number };
/** What preview_formatted_pdf returns: every page the saved PDF would have, and the first ones drawn. */
export type FormattedPreview = { pageCount: number; pages: PreviewPage[]; missingImages: string[] };

/** How long typing must pause before the preview is drawn again. */
export const PREVIEW_DELAY_MS = 300;

const isPreview = (value: unknown): value is FormattedPreview => {
  if (!value || typeof value !== "object") return false;
  const { pageCount, pages, missingImages } = value as Partial<FormattedPreview>;
  return Number.isInteger(pageCount) && Array.isArray(pages) && Array.isArray(missingImages)
    && pages.every((page) => typeof page?.dataUrl === "string" && page.dataUrl.startsWith("data:image/png;base64,") && page.width > 0 && page.height > 0);
};

/**
 * The preview of `document`, the same document a save sends without its output path. It is drawn again once edits
 * pause, and a reply to an older request is dropped, so the pages always belong to the latest document. No document
 * (nothing typed yet) has no preview.
 */
export const useFormattedPreview = (document: object | null) => {
  const key = document === null ? null : JSON.stringify(document);
  const [preview, setPreview] = useState<FormattedPreview | null>(null);
  const [updating, setUpdating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const latest = useRef(0);

  useEffect(() => {
    const request = ++latest.current;
    setError(null);
    if (key === null) {
      setPreview(null);
      setUpdating(false);
      return;
    }
    setUpdating(true);
    const timer = window.setTimeout(() => {
      const current = () => request === latest.current;
      invoke("preview_formatted_pdf", { request: { document: JSON.parse(key) } })
        .then((result) => {
          if (!current()) return;
          if (!isPreview(result)) throw new Error("The preview could not be drawn.");
          setPreview(result);
        })
        .catch((reason) => { if (current()) setError(reason instanceof Error ? reason.message : String(reason)); })
        .finally(() => { if (current()) setUpdating(false); });
    }, PREVIEW_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [key]);

  return { preview: key === null ? null : preview, updating, error };
};

/** The preview pane: the drawn page, which page it is, and whether a newer one is on its way. */
export const FormattedPdfPreview = ({ preview, updating, error, pageName, pageSize }: {
  preview: FormattedPreview | null;
  updating: boolean;
  error: string | null;
  /** The page size and orientation, as "A4 portrait". */
  pageName: string;
  /** The page's width and height in points, for the blank page shown before anything is drawn. */
  pageSize: readonly [number, number];
}) => {
  const [index, setIndex] = useState(0);
  const drawn = preview?.pages.length ?? 0;
  const shown = Math.min(index, Math.max(drawn - 1, 0));
  useEffect(() => { if (index !== shown) setIndex(shown); }, [index, shown]);

  const count = preview?.pageCount ?? 0;
  const page = preview?.pages[shown] ?? null;
  const blank = !updating && !error && count === 0;
  const label = page ? `Page ${shown + 1} of ${count}, ${pageName}` : `Blank ${pageName} page`;
  const status = updating ? "Updating…" : error ? "Preview unavailable" : "";

  return (
    <Card className="formatted-pdf-preview py-0">
      <CardContent className="formatted-pdf-preview-content">
        <section aria-label="Preview" className="formatted-pdf-preview-section">
          <div className="formatted-pdf-preview-bar">
            <span className="workspace-panel-label">Preview</span>
            <span role="status" aria-live="polite" className="workspace-note">{status}</span>
            <span className="formatted-pdf-preview-nav">
              <Button variant="ghost" size="icon" aria-label="Previous page" disabled={shown === 0} onClick={() => setIndex(shown - 1)}><ChevronLeft /></Button>
              <span className="workspace-note tabular-nums">{`${shown + 1} of ${Math.max(count, 1)}`}</span>
              <Button variant="ghost" size="icon" aria-label="Next page" disabled={shown >= drawn - 1} onClick={() => setIndex(shown + 1)}><ChevronRight /></Button>
            </span>
          </div>
          <div className="formatted-pdf-page-frame">
            <div
              role="img"
              aria-label={label}
              aria-busy={updating}
              className="formatted-pdf-page"
              data-updating={updating || undefined}
              style={{ aspectRatio: page ? `${page.width} / ${page.height}` : `${pageSize[0]} / ${pageSize[1]}` }}
            >
              {page && <img src={page.dataUrl} alt="" width={page.width} height={page.height} draggable={false} />}
            </div>
            {updating && (
              <span className="formatted-pdf-preview-chip" aria-hidden="true">
                <LoaderCircle className="animate-spin" />
                Updating preview…
              </span>
            )}
          </div>
          {blank && <p className="workspace-note">Nothing to preview yet. The page fills in as you type.</p>}
          {error && !updating && <p className="workspace-note">{error}</p>}
          {preview && drawn < count && <p className="workspace-note">The preview shows the first {drawn} pages. Saving makes all {count}.</p>}
        </section>
      </CardContent>
    </Card>
  );
};
