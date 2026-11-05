import { Fragment, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { ChevronRight, TriangleAlert } from "lucide-react";
import { UtilityRegistry } from "../registry";
import { isWindows } from "../shortcuts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
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

/** Why a pack was not installed: `PackInstallError` in `src-tauri/src/packs/install.rs`. */
export type PackInstallError = { message: string } & (
  | { kind: "notEnoughSpace"; details: { neededBytes: number; freeBytes: number } }
  | { kind: "wrongTarget"; details: { target: string } }
  | { kind: "overLimit"; details: { limit: "archiveSize" | "unpackedSize" | "entryCount" | "compressionRatio" } }
  | { kind: "unsafeEntry" | "fileMissing" | "fileChecksumMismatch" | "fileSizeMismatch" | "unlistedFile"; details: { path: string } }
  | { kind: "notAPack" | "notInCatalog" | "badSignature" | "wrongKey" | "checksumMismatch" | "malformed" | "incompatibleFormat" | "incompatibleApp" | "unavailable" | "cancelled" | "offline" | "serverBlocked" | "interrupted" | "inUse" }
);

/** A `pack-progress` event from `src-tauri/src/packs/download.rs`. */
export interface PackProgress {
  runId: string;
  packId: string;
  phase: "queued" | "downloading" | "verifying";
  receivedBytes: number;
  totalBytes: number;
}

export const PROGRESS_EVENT = "pack-progress";
/** Sent by `src-tauri/src/packs/install.rs` after a pack is installed, updated or repaired. */
export const PACK_CHANGED_EVENT = "pack-changed";

/** A download started from this view: in progress, or failed with the error its row shows. */
type Job =
  | { runId: string; phase: PackProgress["phase"]; receivedBytes: number; totalBytes: number }
  | { runId: string; phase: "failed"; error: PackInstallError };

const MB = 1024 * 1024;
const GB = 1024 * MB;

export const formatBytes = (bytes: number) => {
  if (bytes >= GB) return `${Number((bytes / GB).toFixed(1))} GB`;
  if (bytes >= MB) return `${Math.round(bytes / MB)} MB`;
  return `${Math.ceil(bytes / 1024)} KB`;
};

/** "7 of 12 MB": the received amount in the total's unit. */
const formatProgress = (received: number, total: number) => {
  const [unit, size, round] = total >= GB
    ? ["GB", GB, (n: number) => Number(n.toFixed(1))] as const
    : total >= MB ? ["MB", MB, Math.round] as const : ["KB", 1024, Math.ceil] as const;
  return `${round(Math.min(received, total) / size)} of ${round(total / size)} ${unit}`;
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

/** What a failed download's row says, from the Packs design board. */
const failedText = (error: PackInstallError) => {
  switch (error.kind) {
    case "serverBlocked":
      return "Couldn't reach the Toolbox release server.";
    case "notEnoughSpace":
      return `Needs ${formatBytes(error.details.neededBytes)} free. ${formatBytes(error.details.freeBytes)} is free on this disk.`;
    case "interrupted":
    case "unavailable":
      return "Download stopped before it finished. Nothing was installed.";
    default:
      return "The download didn't pass its safety check, so it was deleted. Nothing was installed.";
  }
};

/** An uninstall started from this view: waiting for the native side, refused while a run uses the pack, or failed. */
type Removal = "removing" | "inUse" | "failed";

type RowActions = { install: () => void; cancel: (runId: string) => void; remove: () => void; checkForUpdates: () => void };

const PackRow = ({ pack, checking, job, removal, offline, actions }: { pack: PackEntry; checking: boolean; job: Job | undefined; removal: Removal | undefined; offline: boolean; actions: RowActions }) => {
  const [open, setOpen] = useState(false);
  const installed = pack.status !== "notInstalled";
  const tools = toolList(pack.toolIds);
  const download = pack.downloadBytes === null ? null : formatBytes(pack.downloadBytes);
  const offlineRow = offline && !installed && job === undefined;
  // An update downloads beside the installed version, which keeps working until the new one is checked.
  const updating = job !== undefined && job.phase !== "failed" && pack.status === "updateAvailable";
  let status = statusText(pack);
  let state: string = pack.status;
  let bar: { label: string; percent: number } | null = null;
  if (checking) {
    status = "Checking the pack file…";
    state = "checking";
    bar = { label: `Checking ${pack.name}`, percent: 100 };
  } else if (job?.phase === "failed") {
    status = failedText(job.error);
    state = "failed";
  } else if (job?.phase === "queued") {
    status = "Waiting. Starts after the downloads above.";
    state = "queued";
  } else if (job?.phase === "downloading") {
    status = updating ? `Updating, ${formatProgress(job.receivedBytes, job.totalBytes)}` : formatProgress(job.receivedBytes, job.totalBytes);
    state = "downloading";
    bar = { label: `${updating ? "Updating" : "Downloading"} ${pack.name}`, percent: job.totalBytes > 0 ? Math.floor(Math.min(job.receivedBytes / job.totalBytes, 1) * 100) : 0 };
  } else if (job?.phase === "verifying") {
    status = "Checking the download…";
    state = "verifying";
    bar = { label: `Checking ${pack.name}`, percent: 100 };
  } else if (removal === "inUse") {
    status = `Can't remove it while ${tools} is running. Remove it when the run finishes.`;
    state = "inUse";
  } else if (removal === "failed") {
    status = "Couldn't remove this pack. Try again.";
    state = "failed";
  } else if (offlineRow) {
    status = "No internet connection.";
    state = "offline";
  }
  const sub = updating
    ? `Used by ${tools}. Version ${pack.installedVersion} keeps working until ${pack.version} is ready.`
    : installed
    ? `Used by ${tools}. ${formatBytes(pack.diskBytes)} on this disk.`
    : `Used by ${tools}. ${download} download.`;
  const detailsId = `pack-details-${pack.id}`;
  const action = (label: string, onClick: () => void, disabled = false, variant: "outline" | "ghost" = "outline") => (
    <Button variant={variant} size="xs" aria-label={`${label} ${pack.name}`} disabled={disabled} onClick={onClick}>{label}</Button>
  );
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
        <span className="pack-row-end">
          {bar && (
            <span role="progressbar" aria-label={bar.label} aria-valuenow={bar.percent} aria-valuemin={0} aria-valuemax={100} className="pack-progress">
              <span style={{ width: `${bar.percent}%` }} />
            </span>
          )}
          <span className="pack-row-status settings-row-hint" role={state === "inUse" || state === "damaged" ? "alert" : "status"} data-status={state}>{status}</span>
          {job && job.phase !== "failed" && action("Cancel", () => actions.cancel(job.runId))}
          {job?.phase === "failed" && action("Try again", actions.install)}
          {!job && !checking && pack.status === "notInstalled" && action("Install", actions.install, offlineRow)}
          {!job && !checking && pack.status === "updateAvailable" && action("Update", actions.install, offline)}
          {!job && !checking && pack.status === "damaged" && download !== null && action("Repair", actions.install, offline)}
          {!job && !checking && pack.status === "incompatible" && action("Check for updates", actions.checkForUpdates)}
          {!job && !checking && installed && action("Remove…", actions.remove, removal === "removing" || removal === "inUse", "ghost")}
        </span>
      </div>
      {open && (
        <dl className="pack-details" id={detailsId}>
          <dt>What it does</dt><dd>{pack.description}</dd>
          <dt>Used by</dt><dd>{tools}</dd>
          {download !== null && <><dt>Download</dt><dd>{download}</dd></>}
          <dt>On this disk</dt><dd>{installed ? formatBytes(pack.diskBytes) : "Nothing yet"}</dd>
          <dt>Version</dt><dd>{updating ? `${pack.installedVersion} (in use until ${pack.version} is checked)` : versionText(pack)}</dd>
          <dt>License</dt><dd>{pack.license}</dd>
        </dl>
      )}
    </li>
  );
};

/** The note above the list for a problem that affects every download, from the Packs design board. */
const Banner = ({ role, tone, title, text, action }: { role: "status" | "alert"; tone: "warning" | "destructive"; title: string; text: string; action?: { label: string; onClick: () => void } }) => (
  <Card className="settings-note packs-banner py-0" role={role} data-tone={tone}>
    <CardContent className="settings-note-content">
      <TriangleAlert aria-hidden="true" />
      <p><strong>{title}</strong> {text}</p>
      {action && <Button variant="outline" size="xs" onClick={action.onClick}>{action.label}</Button>}
    </CardContent>
  </Card>
);

/**
 * Lists packs, downloads and installs them one at a time, installs one from a pack file, updates
 * them when the built-in catalog offers a newer version, repairs damaged ones by installing them
 * again, and removes them. Nothing is downloaded until the user asks. An incompatible pack sends the
 * user to Updates, since only a newer Toolbox can use it.
 */
export const PacksSection = ({ onCheckForUpdates }: { onCheckForUpdates: () => void }) => {
  const request = useRef<Promise<PackList> | null>(null);
  const [list, setList] = useState<PackList | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [fileStep, setFileStep] = useState<FileStep | null>(null);
  const [checking, setChecking] = useState<string | null>(null);
  const [jobs, setJobs] = useState<Record<string, Job>>({});
  const [removals, setRemovals] = useState<Record<string, Removal>>({});
  const [offline, setOffline] = useState(false);
  useEffect(() => {
    // One request per visit, even when StrictMode runs this effect twice.
    request.current ??= invoke<PackList>("list_packs");
    let live = true;
    request.current
      .then((result) => { if (live) setList(result); })
      .catch(() => { if (live) setError("Could not load packs."); });
    return () => { live = false; };
  }, []);

  useEffect(() => {
    let live = true;
    let stop: UnlistenFn | null = null;
    let stopChanged: UnlistenFn | null = null;
    listen<PackProgress>(PROGRESS_EVENT, ({ payload }) => {
      setJobs((current) => {
        const job = current[payload.packId];
        if (job?.runId !== payload.runId || job.phase === "failed") return current;
        return { ...current, [payload.packId]: { runId: job.runId, phase: payload.phase, receivedBytes: payload.receivedBytes, totalBytes: payload.totalBytes } };
      });
    })
      .then((unlisten) => { if (live) stop = unlisten; else unlisten(); })
      .catch(() => {
        // Without progress events a download still finishes and shows its final state.
      });
    // A pack installed, updated or repaired elsewhere, such as from a result row, shows here at once.
    listen(PACK_CHANGED_EVENT, () => {
      invoke<PackList>("list_packs").then((result) => { if (live) setList(result); }).catch(() => undefined);
    })
      .then((unlisten) => { if (live) stopChanged = unlisten; else unlisten(); })
      .catch(() => undefined);
    // Offline rows wait for the network to come back before offering Install again.
    const online = () => setOffline(false);
    window.addEventListener("online", online);
    return () => {
      live = false;
      stop?.();
      stopChanged?.();
      window.removeEventListener("online", online);
    };
  }, []);

  const download = async (pack: PackEntry) => {
    const runId = crypto.randomUUID();
    setJobs((current) => ({ ...current, [pack.id]: { runId, phase: "downloading", receivedBytes: 0, totalBytes: pack.downloadBytes ?? 0 } }));
    let failure: PackInstallError | null = null;
    try {
      setList(await invoke<PackList>("install_pack", { packId: pack.id, runId }));
      setOffline(false);
    } catch (reason) {
      failure = asInstallError(reason);
      if (failure.kind === "offline") setOffline(true);
    }
    setJobs((current) => {
      const { [pack.id]: finished, ...rest } = current;
      if (finished?.runId !== runId) return current;
      // Cancelled and offline downloads leave the row as it was; offline shows on every row.
      return failure && failure.kind !== "cancelled" && failure.kind !== "offline" ? { ...rest, [pack.id]: { runId, phase: "failed", error: failure } } : rest;
    });
  };

  const remove = async (pack: PackEntry) => {
    setRemovals((current) => ({ ...current, [pack.id]: "removing" }));
    let outcome: Removal | null = null;
    try {
      setList(await invoke<PackList>("uninstall_pack", { packId: pack.id }));
    } catch (reason) {
      // A refusal changes nothing on disk, so the row keeps its state and says why.
      outcome = asInstallError(reason).kind === "inUse" ? "inUse" : "failed";
    }
    setRemovals((current) => {
      const { [pack.id]: _finished, ...rest } = current;
      return outcome ? { ...rest, [pack.id]: outcome } : rest;
    });
  };

  const cancel = (runId: string) => {
    void invoke<boolean>("cancel_run", { runId });
  };

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
  const failures = (list?.packs ?? []).flatMap((pack) => {
    const job = jobs[pack.id];
    return job?.phase === "failed" ? [{ pack, error: job.error }] : [];
  });
  const blocked = failures.some(({ error }) => error.kind === "serverBlocked");
  const inUse = (list?.packs ?? []).find((pack) => removals[pack.id] === "inUse");
  const lowSpace = failures.flatMap(({ pack, error }) => error.kind === "notEnoughSpace" ? [{ pack, ...error.details }] : [])[0];
  // A damaged pack whose repair is not running. Its row says why a repair failed.
  const damaged = (list?.packs ?? []).find((pack) => pack.status === "damaged" && (jobs[pack.id] === undefined || jobs[pack.id].phase === "failed") && checking !== pack.id);
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
      {inUse ? (
        <Banner role="alert" tone="warning" title={`${inUse.name} wasn't removed.`} text={`${toolList(inUse.toolIds)} is using it right now. Remove it after the run finishes.`} />
      ) : offline ? (
        <Banner role="status" tone="warning" title="You're offline." text="Installed packs keep working. Connect to the internet to install or update packs, or use Install from file." />
      ) : blocked ? (
        <Banner
          role="alert"
          tone="destructive"
          title="Couldn't reach the Toolbox release server."
          text="Your internet works, but something on this network is blocking downloads from Toolbox. Try again later or on another network, or install from a pack file."
          action={checking === null ? { label: "Install from file…", onClick: () => void chooseFile() } : undefined}
        />
      ) : lowSpace ? (
        <Banner
          role="alert"
          tone="destructive"
          title={`Not enough space for ${lowSpace.pack.name}.`}
          text={`It needs ${formatBytes(lowSpace.neededBytes)} free and this disk has ${formatBytes(lowSpace.freeBytes)}. Free up at least ${formatBytes(lowSpace.neededBytes - lowSpace.freeBytes)}, then try again.`}
        />
      ) : damaged ? (
        <Banner
          role="alert"
          tone="destructive"
          title={`${damaged.name} is damaged.`}
          text={damaged.downloadBytes === null
            ? `${toolList(damaged.toolIds)} won't use it until it's repaired. Install it again from a pack file.`
            : `${toolList(damaged.toolIds)} won't use it until it's repaired. Repair downloads a fresh copy, ${formatBytes(damaged.downloadBytes)}.`}
          action={damaged.downloadBytes === null
            ? (checking === null ? { label: "Install from file…", onClick: () => void chooseFile() } : undefined)
            : { label: "Repair", onClick: () => void download(damaged) }}
        />
      ) : null}
      <ul className="pack-list" aria-label="Packs">
        {list?.packs.map((pack) => (
          <PackRow
            key={pack.id}
            pack={pack}
            checking={checking === pack.id}
            job={jobs[pack.id]}
            removal={removals[pack.id]}
            offline={offline}
            actions={{ install: () => void download(pack), cancel, remove: () => void remove(pack), checkForUpdates: onCheckForUpdates }}
          />
        ))}
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
