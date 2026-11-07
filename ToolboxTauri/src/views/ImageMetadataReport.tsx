import type { ImageMetadataInspection } from "../contracts";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

type Report = Extract<ImageMetadataInspection, { status: "report" }>;
type Fate = "Removed" | "Kept" | "None found";
type Row = { name: string; detail: string; fate: Fate };

const orientationDetails: Record<number, string> = {
  1: "Upright, no turn needed",
  2: "Flip horizontally to show upright",
  3: "Rotate 180° to show upright",
  4: "Flip vertically to show upright",
  5: "Rotate 90° clockwise and flip horizontally to show upright",
  6: "Rotate 90° clockwise to show upright",
  7: "Rotate 90° clockwise and flip vertically to show upright",
  8: "Rotate 90° counter-clockwise to show upright",
};

const removed = (name: string, present: boolean, detail: string): Row =>
  present ? { name, detail, fate: "Removed" } : { name, detail: "None found", fate: "None found" };

const kept = (name: string, present: boolean, detail: string): Row =>
  present ? { name, detail, fate: "Kept" } : { name, detail: "None found", fate: "None found" };

/** Labelled categories in the order the Edit Image board shows them. */
export const metadataRows = (report: Report): Row[] => [
  removed("Camera", report.exif, "Camera details (EXIF)"),
  removed("Location", report.gps, "GPS location"),
  removed("XMP", report.xmp, "XMP metadata"),
  removed("IPTC", report.iptc, "IPTC metadata"),
  removed("Comments", report.comments, "Embedded comments"),
  kept("Orientation", report.orientation !== null, report.orientation === null ? "" : orientationDetails[report.orientation] ?? `Orientation ${report.orientation}`),
  kept("Colour profile", report.icc, "Embedded colour profile"),
];

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

export const ImageMetadataReport = ({ inspections, error }: { inspections: ImageMetadataInspection[]; error?: string }) => (
  <section className="image-metadata-report" aria-label="Metadata found">
    {error && <p className="workspace-note" role="alert">Metadata could not be inspected. {error}</p>}
    {inspections.map((inspection, index) => {
      const name = fileName(inspection.path);
      if (inspection.status === "error") {
        return (
          <Card key={inspection.path + index} className="gap-2 py-3" role="region" aria-label={`Metadata in ${name}`}>
            <CardHeader className="px-3">
              <CardTitle className="text-sm">{name}</CardTitle>
            </CardHeader>
            <CardContent className="px-3">
              <p className="workspace-note" role="alert">Could not read this image's metadata. {inspection.message}</p>
            </CardContent>
          </Card>
        );
      }
      return (
        <Card key={inspection.path + index} className="gap-2 py-3" role="region" aria-label={`Metadata in ${name}`}>
          <CardHeader className="px-3">
            <CardTitle className="text-sm">{name}</CardTitle>
            <p className="workspace-note"><strong>Found in this image</strong> Nothing changes until you run.</p>
          </CardHeader>
          <CardContent className="px-3">
            <dl className="image-metadata-rows">
              {metadataRows(inspection).map((row) => (
                <div key={row.name} className="image-metadata-row" data-present={row.fate !== "None found" ? "true" : "false"}>
                  <dt>{row.name}</dt>
                  <dd className="workspace-note">{row.detail}</dd>
                  <dd><Badge variant="outline">{row.fate}</Badge></dd>
                </div>
              ))}
            </dl>
          </CardContent>
        </Card>
      );
    })}
  </section>
);
