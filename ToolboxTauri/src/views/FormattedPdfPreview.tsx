import { useEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronLeft, ChevronRight, LoaderCircle, Pause, TriangleAlert } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { packVerbs } from "../components/resultRow";
import { formatBytes } from "./PacksSection";
import type { PackNeed } from "../contracts";

/** One drawn page, as preview_formatted_pdf returns it. */
export type PreviewPage = { dataUrl: string; width: number; height: number };
/** What preview_formatted_pdf returns: every page the saved PDF would have, and the first ones drawn. */
export type FormattedPreview = { pageCount: number; pages: PreviewPage[]; missingImages: string[] };

/**
 * A preview refused because text in `script` needs a pack that isn't usable, from the error's pack pointer. Saving is
 * refused the same way, before anything is written.
 */
export type ScriptRefusal = { script: string; packId: string; packName: string; reason: string };

/** The verb that makes `refusal`'s pack usable: Install, Repair or Update. */
export const packVerbOf = (refusal: ScriptRefusal) => packVerbs[refusal.reason as PackNeed] ?? packVerbs.notInstalled;

/** How long typing must pause before the preview is drawn again. */
export const PREVIEW_DELAY_MS = 300;

const isPreview = (value: unknown): value is FormattedPreview => {
  if (!value || typeof value !== "object") return false;
  const { pageCount, pages, missingImages } = value as Partial<FormattedPreview>;
  return Number.isInteger(pageCount) && Array.isArray(pages) && Array.isArray(missingImages)
    && pages.every((page) => typeof page?.dataUrl === "string" && page.dataUrl.startsWith("data:image/png;base64,") && page.width > 0 && page.height > 0);
};

/** A failed preview's wording, and the pack it needs when a script's pack is missing. */
const problemOf = (reason: unknown): { error: string; refusal: ScriptRefusal | null } => {
  if (reason instanceof Error) return { error: reason.message, refusal: null };
  if (!reason || typeof reason !== "object") return { error: String(reason), refusal: null };
  const { message, script, pointer } = reason as { message?: unknown; script?: unknown; pointer?: { kind?: unknown; packId?: unknown; name?: unknown; reason?: unknown } | null };
  const refusal = typeof script === "string" && pointer?.kind === "pack" && typeof pointer.packId === "string" && typeof pointer.name === "string"
    ? { script, packId: pointer.packId, packName: pointer.name, reason: typeof pointer.reason === "string" ? pointer.reason : "notInstalled" }
    : null;
  return { error: typeof message === "string" ? message : "The preview could not be drawn.", refusal };
};

/**
 * The preview of `document`, the same document a save sends without its output path. It is drawn again once edits
 * pause, and a reply to an older request is dropped, so the pages always belong to the latest document. No document
 * (nothing typed yet) has no preview. A changed `refresh`, as after a pack is installed, draws it again. A failure
 * keeps the last pages and stays until the next reply, so a refusal doesn't flicker while typing.
 */
export const useFormattedPreview = (document: object | null, refresh = 0) => {
  const key = document === null ? null : JSON.stringify(document);
  const [preview, setPreview] = useState<FormattedPreview | null>(null);
  const [updating, setUpdating] = useState(false);
  const [problem, setProblem] = useState<{ error: string; refusal: ScriptRefusal | null } | null>(null);
  const latest = useRef(0);

  useEffect(() => {
    const request = ++latest.current;
    if (key === null) {
      setPreview(null);
      setProblem(null);
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
          setProblem(null);
        })
        .catch((reason) => { if (current()) setProblem(problemOf(reason)); })
        .finally(() => { if (current()) setUpdating(false); });
    }, PREVIEW_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [key, refresh]);

  return { preview: key === null ? null : preview, updating, error: problem?.error ?? null, refusal: problem?.refusal ?? null };
};

/**
 * The preview pane: the drawn page, which page it is, and whether a newer one is on its way. `paused` is the note
 * shown while the preview can't be drawn until a script pack is installed; the last pages stay, dimmed.
 */
export const FormattedPdfPreview = ({ preview, updating, error, paused, pageName, pageSize }: {
  preview: FormattedPreview | null;
  updating: boolean;
  error: string | null;
  paused: ReactNode | null;
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
  const blank = !updating && !error && !paused && count === 0;
  const label = page ? `Page ${shown + 1} of ${count}, ${pageName}` : `Blank ${pageName} page`;
  const status = updating ? "Updating…" : paused ? "Paused" : error ? "Preview unavailable" : "";

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
          {paused}
          <div className="formatted-pdf-page-frame">
            <div
              role="img"
              aria-label={label}
              aria-busy={updating}
              className="formatted-pdf-page"
              data-updating={updating || paused !== null || undefined}
              style={{ aspectRatio: page ? `${page.width} / ${page.height}` : `${pageSize[0]} / ${pageSize[1]}` }}
            >
              {page && <img src={page.dataUrl} alt="" width={page.width} height={page.height} draggable={false} />}
            </div>
            {updating ? (
              <span className="formatted-pdf-preview-chip" aria-hidden="true">
                <LoaderCircle className="animate-spin" />
                Updating preview…
              </span>
            ) : paused && (
              <span className="formatted-pdf-preview-chip" aria-hidden="true">
                <Pause />
                Preview paused
              </span>
            )}
          </div>
          {blank && <p className="workspace-note">Nothing to preview yet. The page fills in as you type.</p>}
          {error && !updating && !paused && <p className="workspace-note">{error}</p>}
          {preview && drawn < count && <p className="workspace-note">The preview shows the first {drawn} pages. Saving makes all {count}.</p>}
        </section>
      </CardContent>
    </Card>
  );
};

/**
 * The preview's note while a script's pack is missing, from the board's "Script pack needed" state. Nothing downloads
 * until Install is chosen.
 */
export const ScriptPackNote = ({ refusal, block, downloadBytes, downloading, installError, onInstall }: {
  refusal: ScriptRefusal;
  /** The editor block that has the text, or null when it's in the header or footer. */
  block: number | null;
  downloadBytes: number | null;
  /** The install's progress, 0 to 100 or null when its size isn't known yet, or undefined when it isn't running. */
  downloading: number | null | undefined;
  installError: string | null;
  onInstall: () => void;
}) => {
  const verb = packVerbOf(refusal);
  return (
    <Card role="status" className="formatted-pdf-pack-note gap-0 py-0">
      <CardContent className="formatted-pdf-alert-content">
        <TriangleAlert aria-hidden="true" />
        <p>
          <strong>{refusal.script} text needs the {refusal.packName} pack</strong>
          <span>{verb} it to preview and save {block === null ? "the document" : `block ${block}`}, or remove that text. Nothing downloads until you choose {verb}.</span>
          {downloadBytes !== null && (
            <span className="formatted-pdf-pack-action">
              <Button variant="outline" size="sm" disabled={downloading !== undefined} onClick={onInstall}>{verb} script pack</Button>
              <span>{downloading === undefined ? `${formatBytes(downloadBytes)} download` : `Downloading${downloading === null ? "…" : ` ${downloading}%`}`}</span>
            </span>
          )}
          {installError && <span role="alert">{installError}</span>}
        </p>
      </CardContent>
    </Card>
  );
};
