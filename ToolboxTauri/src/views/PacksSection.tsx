import { Fragment, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { ChevronRight } from "lucide-react";
import { UtilityRegistry } from "../registry";
import { isWindows } from "../shortcuts";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";

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

/** What a pack file says it is, matched to the catalog, before anything in it is trusted. */
export interface PackFileSummary {
  fileName: string;
  packId: string;
  name: string;
  version: string;
  installedBytes: number;
  license: string;
}

/** Why a pack file was not installed: `PackInstallError` in `src-tauri/src/packs/install.rs`. */
export type PackInstallError = { message: string } & (
  | { kind: "wrongTarget"; details: { target: string } }
  | { kind: "overLimit"; details: { limit: "archiveSize" | "unpackedSize" | "entryCount" | "compressionRatio" } }
  | { kind: "unsafeEntry" | "fileMissing" | "fileChecksumMismatch" | "fileSizeMismatch" | "unlistedFile"; details: { path: string } }
  | { kind: "notAPack" | "notInCatalog" | "badSignature" | "wrongKey" | "checksumMismatch" | "malformed" | "incompatibleFormat" | "incompatibleApp" | "unavailable" }
);

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

const RELEASES = "the Toolbox releases page";

const madeFor = (target: string) => {
  const [os] = target.split("-");
  if (os === "macos") return isWindows() ? "a Mac" : "a Mac with a different processor";
  if (os === "windows") return isWindows() ? "Windows on a different processor" : "Windows";
  return "another kind of computer";
};

/** The plain-language copy for each install error, from the Packs design board. */
const errorCopy = (error: PackInstallError): { title: string; text: string } => {
  switch (error.kind) {
    case "wrongTarget":
      return {
        title: "This pack file is for another kind of computer",
        text: `It was made for ${madeFor(error.details.target)}, so it can't be installed here. Nothing was installed. Get the pack file for ${isWindows() ? "Windows on this processor" : "Mac"} from ${RELEASES}.`,
      };
    case "notAPack":
    case "badSignature":
    case "wrongKey":
      return {
        title: "This file isn't a Toolbox pack",
        text: `It isn't signed by Toolbox, so it can't be installed. Nothing was installed. Only use pack files from ${RELEASES}.`,
      };
    case "notInCatalog":
    case "incompatibleFormat":
    case "incompatibleApp":
      return {
        title: "This pack file doesn't work with this version of Toolbox",
        text: `It was made for a different version of Toolbox, so it can't be installed. Nothing was installed. Get the pack file for this version from ${RELEASES}.`,
      };
    case "unavailable":
      return {
        title: "Toolbox couldn't install this pack",
        text: "Toolbox couldn't save the pack on this computer. Nothing was installed. Check that there's free disk space, then try again.",
      };
    default:
      return {
        title: "This pack file didn't pass its safety check",
        text: `It may be damaged or changed since it was made, so it was not installed. Copy the file again from ${RELEASES}, then try again.`,
      };
  }
};

/** A rejection from a pack command, or `unavailable` for anything the native side did not type. */
const asInstallError = (reason: unknown): PackInstallError =>
  typeof reason === "object" && reason !== null && "kind" in reason
    ? reason as PackInstallError
    : { kind: "unavailable", message: String(reason) };

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

type FileStep =
  | { step: "confirm"; path: string; summary: PackFileSummary }
  | { step: "error"; fileName: string; error: PackInstallError };

const PackFacts = ({ facts }: { facts: Array<[string, string]> }) => (
  <dl className="pack-facts">
    {facts.map(([term, value]) => <Fragment key={term}><dt>{term}</dt><dd>{value}</dd></Fragment>)}
  </dl>
);

const PackRow = ({ pack, checking }: { pack: PackEntry; checking: boolean }) => {
  const [open, setOpen] = useState(false);
  const installed = pack.status !== "notInstalled";
  const tools = toolList(pack.toolIds);
  const download = pack.downloadBytes === null ? null : formatBytes(pack.downloadBytes);
  const status = checking ? "Checking the pack file…" : statusText(pack);
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
        <span className="pack-row-status settings-row-hint" role="status" data-status={checking ? "checking" : pack.status}>
          {checking && (
            <span role="progressbar" aria-label={`Checking ${pack.name}`} aria-valuenow={100} aria-valuemin={0} aria-valuemax={100} className="pack-progress">
              <span />
            </span>
          )}
          {status}
        </span>
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

/** Lists packs and installs one from a pack file. Downloading, updating and removing arrive later. */
export const PacksSection = () => {
  const request = useRef<Promise<PackList> | null>(null);
  const [list, setList] = useState<PackList | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [fileStep, setFileStep] = useState<FileStep | null>(null);
  const [checking, setChecking] = useState<string | null>(null);
  useEffect(() => {
    // One request per visit, even when StrictMode runs this effect twice.
    request.current ??= invoke<PackList>("list_packs");
    let live = true;
    request.current
      .then((result) => { if (live) setList(result); })
      .catch(() => { if (live) setError("Could not load packs."); });
    return () => { live = false; };
  }, []);

  const chooseFile = async () => {
    setFileStep(null);
    const path = await open({ multiple: false, directory: false, filters: [{ name: "Toolbox pack", extensions: ["toolboxpack"] }] });
    if (typeof path !== "string") return;
    try {
      const summary = await invoke<PackFileSummary>("inspect_pack_file", { path });
      setFileStep({ step: "confirm", path, summary });
    } catch (reason) {
      setFileStep({ step: "error", fileName: fileName(path), error: asInstallError(reason) });
    }
  };

  const install = async (path: string, summary: PackFileSummary) => {
    setFileStep(null);
    setChecking(summary.packId);
    try {
      setList(await invoke<PackList>("install_pack_file", { path }));
    } catch (reason) {
      setFileStep({ step: "error", fileName: summary.fileName, error: asInstallError(reason) });
    } finally {
      setChecking(null);
    }
  };

  const copy = fileStep?.step === "error" ? errorCopy(fileStep.error) : null;
  return (
    <div className="packs">
      <div className="packs-intro">
        <div className="packs-intro-text">
          <h2 id="packs-heading" className="settings-row-title">Optional packs</h2>
          <p className="settings-row-hint">
            Packs add file types and languages. Each one downloads once from the Toolbox release server, is checked before it's used, and then works offline. Your files never leave this device.
          </p>
        </div>
        <Button variant="outline" size="xs" disabled={checking !== null} onClick={() => void chooseFile()}>Install from file…</Button>
      </div>
      <ul className="pack-list" aria-label="Packs">
        {list?.packs.map((pack) => <PackRow key={pack.id} pack={pack} checking={checking === pack.id} />)}
      </ul>
      {list && (
        <p className="settings-row-hint" role="status">
          {list.installedCount === 0
            ? "No packs installed."
            : `${list.installedCount} installed, ${formatBytes(list.totalBytes)} used.`}
        </p>
      )}
      {error && <p className="settings-update-status" role="alert">{error}</p>}
      {fileStep?.step === "confirm" && (
        <Dialog open onOpenChange={(next) => { if (!next) setFileStep(null); }}>
          <DialogContent showCloseButton={false} className="sm:max-w-[440px]">
            <DialogHeader>
              <DialogTitle>Install this pack?</DialogTitle>
              <DialogDescription>
                Toolbox checks the file's signature and contents before installing it. Nothing is downloaded, and your files never leave this device.
              </DialogDescription>
            </DialogHeader>
            <PackFacts facts={[
              ["File", fileStep.summary.fileName],
              ["Pack", fileStep.summary.name],
              ["Version", fileStep.summary.version],
              ["Size", `${formatBytes(fileStep.summary.installedBytes)} on this disk`],
              ["License", fileStep.summary.license],
            ]} />
            <DialogFooter>
              <Button variant="ghost" onClick={() => setFileStep(null)}>Cancel</Button>
              <Button onClick={() => void install(fileStep.path, fileStep.summary)}>Install</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
      {fileStep?.step === "error" && copy && (
        <Dialog open onOpenChange={(next) => { if (!next) setFileStep(null); }}>
          <DialogContent role="alertdialog" showCloseButton={false} className="sm:max-w-[440px]">
            <DialogHeader>
              <DialogTitle>{copy.title}</DialogTitle>
              <DialogDescription>{copy.text}</DialogDescription>
            </DialogHeader>
            <PackFacts facts={[["File", fileStep.fileName]]} />
            <DialogFooter>
              <Button variant="ghost" onClick={() => void chooseFile()}>Choose another file…</Button>
              <Button onClick={() => setFileStep(null)}>OK</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </div>
  );
};
