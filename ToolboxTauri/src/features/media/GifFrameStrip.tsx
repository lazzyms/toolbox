import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ImagePreview } from "../../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";

/** The still images of a new GIF, in output order, with a preview of each. */
export const GifFrameStrip = ({
  files,
  supportsPreview,
  selectedIndex,
  selectIndex,
}: {
  files: string[];
  supportsPreview: boolean;
  selectedIndex: number;
  selectIndex: (index: number) => void;
}) => {
  const [previews, setPreviews] = useState<Record<string, ImagePreview | null>>({});
  const frameKey = files.join("\u0000");

  useEffect(() => {
    if (!frameKey || !supportsPreview) {
      setPreviews({});
      return;
    }
    let current = true;
    setPreviews({});
    void Promise.all(
      files.map(async (path) => {
        try {
          return [path, await invoke<ImagePreview>("inspect_image_preview", { request: { path } })] as const;
        } catch {
          return [path, null] as const;
        }
      }),
    ).then((entries) => {
      if (current) setPreviews(Object.fromEntries(entries));
    });
    return () => {
      current = false;
    };
    // The frame key stands for the files, so a new array with the same paths does not refetch.
  }, [frameKey, supportsPreview]);

  return (
    <Card className="media-frame-order py-0" role="region" aria-label="Frame order">
      <CardContent className="media-frame-order-content py-4">
      <p className="workspace-panel-label">Frame order</p>
      <p className="workspace-panel-copy">The output follows this order. Select a frame, then move it with the arrows.</p>
      <ol className="media-frame-list">
        {files.map((file, index) => {
          const preview = previews[file];
          return (
            <li key={file} data-selected={index === selectedIndex ? "true" : undefined}>
              <Button variant={index === selectedIndex ? "secondary" : "ghost"} size="sm" aria-pressed={index === selectedIndex} onClick={() => selectIndex(index)} aria-label={"Frame " + (index + 1)}>
                <span>{index + 1}</span>
                {supportsPreview && preview && <img className="media-frame-preview" src={preview.dataUrl} alt={"Preview of frame " + (index + 1)} />}
                {file.split(/[\\/]/).pop()}
              </Button>
            </li>
          );
        })}
      </ol>
      {files.length === 0 && <p className="workspace-note">Select frames to arrange them here.</p>}
      </CardContent>
    </Card>
  );
};
