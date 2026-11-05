import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronRight } from "lucide-react";
import { UtilityRegistry } from "../registry";
import { Button } from "@/components/ui/button";

export type PackStatus = "notInstalled" | "installed" | "updateAvailable" | "damaged" | "incompatible";

export interface PackEntry {
  id: string;
  name: string;
  description: string;
  toolIds: string[];
  license: string;
  version: string;
  downloadBytes: number | null;
  installedBytes: number | null;
  diskBytes: number;
  status: PackStatus;
  installedVersion: string | null;
}

export interface PackList {
  packs: PackEntry[];
  installedCount: number;
  totalBytes: number;
}

const MB = 1024 * 1024;
const GB = 1024 * MB;

export const formatBytes = (bytes: number) => {
  if (bytes >= GB) return `${Number((bytes / GB).toFixed(1))} GB`;
  if (bytes >= MB) return `${Math.round(bytes / MB)} MB`;
  return `${Math.ceil(bytes / 1024)} KB`;
};

const toolTitles = new Map<string, string>(UtilityRegistry.map(({ id, title }) => [id, title]));
const toolList = (toolIds: string[]) =>
  new Intl.ListFormat("en", { type: "conjunction" }).format(toolIds.map((id) => toolTitles.get(id) ?? id));

const statusText = (pack: PackEntry) => {
  switch (pack.status) {
    case "notInstalled":
      return "Not installed";
    case "installed":
      return `Installed, version ${pack.installedVersion}`;
    case "updateAvailable":
      return `Version ${pack.installedVersion}. Update to ${pack.version}${pack.downloadBytes === null ? "" : `, ${formatBytes(pack.downloadBytes)}`}`;
    case "damaged":
      return "Damaged. A file in this pack was changed or is missing.";
    case "incompatible":
      return "Doesn't work with this version of Toolbox. Update Toolbox to use it.";
  }
};

const versionText = (pack: PackEntry) => {
  if (pack.status === "notInstalled") return pack.version;
  if (pack.status === "updateAvailable") return `${pack.installedVersion}, ${pack.version} is ready`;
  return pack.installedVersion ?? "Unknown";
};

const PackRow = ({ pack }: { pack: PackEntry }) => {
  const [open, setOpen] = useState(false);
  const installed = pack.status !== "notInstalled";
  const tools = toolList(pack.toolIds);
  const download = pack.downloadBytes === null ? null : formatBytes(pack.downloadBytes);
  const status = statusText(pack);
  const sub = installed
    ? `Used by ${tools}. ${formatBytes(pack.diskBytes)} on this disk.`
    : `Used by ${tools}. ${download} download.`;
  const detailsId = `pack-details-${pack.id}`;
  return (
    <li className="pack-row" aria-label={`${pack.name}, ${status}`}>
      <div className="pack-row-main">
        <Button
          variant="ghost"
          size="icon-xs"
          aria-expanded={open}
          aria-controls={detailsId}
          aria-label={`Details for ${pack.name}`}
          onClick={() => setOpen((value) => !value)}
        >
          <ChevronRight aria-hidden="true" className="pack-chevron" data-open={open} />
        </Button>
        <span className="pack-row-text">
          <span className="pack-row-name">{pack.name}</span>
          <span className="settings-row-hint">{sub}</span>
        </span>
        <span className="pack-row-status settings-row-hint" role="status" data-status={pack.status}>{status}</span>
      </div>
      {open && (
        <dl className="pack-details" id={detailsId}>
          <dt>What it does</dt><dd>{pack.description}</dd>
          <dt>Used by</dt><dd>{tools}</dd>
          {download !== null && <><dt>Download</dt><dd>{download}</dd></>}
          <dt>On this disk</dt><dd>{installed ? formatBytes(pack.diskBytes) : "Nothing yet"}</dd>
          <dt>Version</dt><dd>{versionText(pack)}</dd>
          <dt>License</dt><dd>{pack.license}</dd>
        </dl>
      )}
    </li>
  );
};

/** Read-only for now: installing, updating and removing packs arrive in later work. */
export const PacksSection = () => {
  const request = useRef<Promise<PackList> | null>(null);
  const [list, setList] = useState<PackList | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    // One request per visit, even when StrictMode runs this effect twice.
    request.current ??= invoke<PackList>("list_packs");
    let live = true;
    request.current
      .then((result) => { if (live) setList(result); })
      .catch(() => { if (live) setError("Could not load packs."); });
    return () => { live = false; };
  }, []);
  return (
    <div className="packs">
      <div className="packs-intro">
        <h2 id="packs-heading" className="settings-row-title">Optional packs</h2>
        <p className="settings-row-hint">
          Packs add file types and languages. Each one downloads once from the Toolbox release server, is checked before it's used, and then works offline. Your files never leave this device.
        </p>
      </div>
      <ul className="pack-list" aria-label="Packs">
        {list?.packs.map((pack) => <PackRow key={pack.id} pack={pack} />)}
      </ul>
      {list && (
        <p className="settings-row-hint" role="status">
          {list.installedCount === 0
            ? "No packs installed."
            : `${list.installedCount} installed, ${formatBytes(list.totalBytes)} used.`}
        </p>
      )}
      {error && <p className="settings-update-status" role="alert">{error}</p>}
    </div>
  );
};
