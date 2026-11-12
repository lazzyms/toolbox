import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Download, TriangleAlert } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue } from "@/components/ui/select";
import { packVerbs } from "../components/resultRow";
import { formatBytes, PACK_CHANGED_EVENT, PROGRESS_EVENT, type PackProgress, type PackStatus } from "./PacksSection";

/** One language OCR can read, as `ocr_languages` reports it: English is built in, the rest are packs. */
export interface OcrLanguage {
  code: string;
  name: string;
  packId: string | null;
  status: PackStatus;
  downloadBytes: number | null;
}

/** The language a fresh install can read before any pack is installed. */
export const BUILT_IN_OCR_LANGUAGE: OcrLanguage = { code: "eng", name: "English", packId: null, status: "installed", downloadBytes: null };

const usable = (language: OcrLanguage) => language.status === "installed" || language.status === "updateAvailable";

const optionLabel = (language: OcrLanguage) => usable(language)
  ? `${language.name}, installed`
  : [language.name, "not installed", language.downloadBytes ? `${formatBytes(language.downloadBytes)} download` : null].filter(Boolean).join(", ");

const statusText = (language: OcrLanguage) => {
  if (usable(language)) return "Installed";
  if (language.status === "damaged") return "Damaged";
  if (language.status === "incompatible") return "Needs an update";
  return "Not installed";
};

type InstallJob = { runId: string; packId: string; receivedBytes: number; totalBytes: number };

export const OcrLanguagePicker = ({ value, onChange }: { value: string; onChange: (code: string) => void }) => {
  const [languages, setLanguages] = useState<OcrLanguage[]>([BUILT_IN_OCR_LANGUAGE]);
  const [job, setJob] = useState<InstallJob | null>(null);
  const [installError, setInstallError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    const refresh = () => invoke<OcrLanguage[]>("ocr_languages")
      .then((list) => { if (live && list.length > 0) setLanguages(list); })
      .catch(() => undefined);
    const stops: UnlistenFn[] = [];
    const keep = (unlisten: UnlistenFn) => { if (live) stops.push(unlisten); else unlisten(); };
    // A pack installed or removed anywhere, here or in Settings, changes what the picker offers.
    listen(PACK_CHANGED_EVENT, () => { void refresh(); }).then(keep).catch(() => undefined);
    listen<PackProgress>(PROGRESS_EVENT, ({ payload }) => {
      setJob((current) => current?.runId === payload.runId
        ? { ...current, receivedBytes: payload.receivedBytes, totalBytes: payload.totalBytes }
        : current);
    }).then(keep).catch(() => undefined);
    void refresh();
    return () => {
      live = false;
      stops.forEach((stop) => stop());
    };
  }, []);

  const chosen = languages.find((language) => language.code === value) ?? BUILT_IN_OCR_LANGUAGE;
  const installed = languages.filter(usable);
  const downloadable = languages.filter((language) => !usable(language));
  const installing = job !== null && job.packId === chosen.packId;
  const verb = chosen.status in packVerbs ? packVerbs[chosen.status as keyof typeof packVerbs] : undefined;
  const canInstall = chosen.packId !== null && verb !== undefined && chosen.downloadBytes !== null;

  const install = async () => {
    if (!chosen.packId) return;
    const runId = crypto.randomUUID();
    setInstallError(null);
    setJob({ runId, packId: chosen.packId, receivedBytes: 0, totalBytes: 0 });
    try {
      await invoke("install_pack", { packId: chosen.packId, runId });
    } catch (reason) {
      // A pack error carries its wording; a cancelled install leaves the language as it was.
      const failure = reason as { kind?: string; message?: string } | null;
      if (failure?.kind !== "cancelled") setInstallError(failure?.message || String(reason));
    } finally {
      setJob((current) => current?.runId === runId ? null : current);
    }
  };

  return (
    <div className="workspace-field grid gap-2">
      <Label htmlFor="ocr-language">Language</Label>
      <Select value={chosen.code} onValueChange={(code) => { setInstallError(null); onChange(code); }}>
        <SelectTrigger id="ocr-language" aria-label="Language" className="w-full">
          <SelectValue>{chosen.name}</SelectValue>
        </SelectTrigger>
        <SelectContent position="popper">
          <SelectGroup>
            <SelectLabel>Installed</SelectLabel>
            {installed.map((language) => (
              <SelectItem key={language.code} value={language.code} aria-label={optionLabel(language)} aria-labelledby={undefined}>{language.name}</SelectItem>
            ))}
          </SelectGroup>
          {downloadable.length > 0 && (
            <SelectGroup>
              <SelectLabel>Download to use</SelectLabel>
              {downloadable.map((language) => (
                <SelectItem key={language.code} value={language.code} aria-label={optionLabel(language)} aria-labelledby={undefined}>
                  {language.name}
                  <Download aria-hidden="true" />
                  {language.downloadBytes ? <span className="text-muted-foreground">{formatBytes(language.downloadBytes)}</span> : null}
                </SelectItem>
              ))}
            </SelectGroup>
          )}
        </SelectContent>
      </Select>
      {installing ? (
        <p className="workspace-note" role="status">
          Downloading {job.totalBytes > 0 ? `${Math.floor((job.receivedBytes / job.totalBytes) * 100)}%` : chosen.name}
        </p>
      ) : (
        <p className="workspace-note" aria-label={`${chosen.name} status`}>{statusText(chosen)}</p>
      )}
      {!usable(chosen) && !installing && (
        <div className="workspace-note flex items-start gap-2">
          <TriangleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
          <div className="grid gap-2">
            {chosen.status === "notInstalled" && chosen.downloadBytes ? (
              <p><strong>{chosen.name} needs a {formatBytes(chosen.downloadBytes)} download.</strong> Toolbox downloads it once, then works offline.</p>
            ) : chosen.status === "damaged" ? (
              <p><strong>{chosen.name} needs repair.</strong> Toolbox downloads it again, then works offline.</p>
            ) : (
              <p><strong>{chosen.name} doesn&apos;t work with this version of Toolbox.</strong>{canInstall ? " Toolbox downloads the version that does." : " Update Toolbox to read it."}</p>
            )}
            {canInstall && (
              <Button variant="outline" size="sm" className="justify-self-start" onClick={() => void install()}>
                {verb} {chosen.name}
              </Button>
            )}
          </div>
        </div>
      )}
      {installError && <p className="workspace-note" role="alert">{installError}</p>}
    </div>
  );
};
