import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ArrowDown, ArrowUp, Check, Download, TriangleAlert, X } from "lucide-react";
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

/** Where the last chosen languages are kept, in the order they go to the engine. */
export const OCR_LANGUAGES_STORAGE_KEY = "toolbox-ocr-languages";

const rememberedLanguages = (): string[] | null => {
  try {
    const stored: unknown = JSON.parse(window.localStorage.getItem(OCR_LANGUAGES_STORAGE_KEY) ?? "null");
    if (!Array.isArray(stored) || !stored.every((code) => typeof code === "string")) return null;
    const codes = [...new Set(stored as string[])];
    return codes.length > 0 ? codes : null;
  } catch {
    return null;
  }
};

const rememberLanguages = (codes: string[]) => {
  try {
    window.localStorage.setItem(OCR_LANGUAGES_STORAGE_KEY, JSON.stringify(codes));
  } catch {
    // Without storage the choice lasts until the app closes.
  }
};

/**
 * The remembered languages that can still be read, in their order, and the names of the
 * ones whose pack is gone. A damaged or outdated pack stays, so it can be repaired.
 * With nothing left, English reads on its own.
 */
const reconcile = (codes: string[], available: OcrLanguage[]) => {
  const byCode = new Map(available.map((language) => [language.code, language]));
  const gone = (code: string) => { const language = byCode.get(code); return !language || language.status === "notInstalled"; };
  const kept = codes.filter((code) => !gone(code));
  const removed = codes.filter(gone).map((code) => byCode.get(code)?.name ?? code);
  return { codes: kept.length > 0 ? kept : [BUILT_IN_OCR_LANGUAGE.code], removed };
};

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

/** "Tamil was removed", "Tamil and Gujarati were removed". */
const removedNotice = (names: string[]) => {
  const one = names.length === 1;
  const list = one ? names[0] : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
  return {
    title: `${list} ${one ? "was" : "were"} removed from your languages.`,
    body: one
      ? "Its pack isn't installed any more. Add it again from the list to download it."
      : "Their packs aren't installed any more. Add them again from the list to download them.",
  };
};

type InstallJob = { runId: string; receivedBytes: number; totalBytes: number };

export const OcrLanguagePicker = ({ value, onChange }: { value: string[]; onChange: (codes: string[]) => void }) => {
  const [languages, setLanguages] = useState<OcrLanguage[]>([BUILT_IN_OCR_LANGUAGE]);
  const [jobs, setJobs] = useState<Record<string, InstallJob>>({});
  const [installError, setInstallError] = useState<string | null>(null);
  const [removed, setRemoved] = useState<string[]>([]);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  useEffect(() => {
    let live = true;
    let reconciled = false;
    const refresh = () => invoke<OcrLanguage[]>("ocr_languages")
      .then((list) => {
        if (!live || list.length === 0) return;
        setLanguages(list);
        // The last choice comes back once the installed languages are known.
        if (reconciled) return;
        reconciled = true;
        const stored = rememberedLanguages();
        if (!stored) return;
        const { codes, removed } = reconcile(stored, list);
        rememberLanguages(codes);
        setRemoved(removed);
        onChangeRef.current(codes);
      })
      .catch(() => undefined);
    const stops: UnlistenFn[] = [];
    const keep = (unlisten: UnlistenFn) => { if (live) stops.push(unlisten); else unlisten(); };
    // A pack installed or removed anywhere, here or in Settings, changes what the picker offers.
    listen(PACK_CHANGED_EVENT, () => { void refresh(); }).then(keep).catch(() => undefined);
    listen<PackProgress>(PROGRESS_EVENT, ({ payload }) => {
      setJobs((current) => {
        const entry = Object.entries(current).find(([, job]) => job.runId === payload.runId);
        return entry ? { ...current, [entry[0]]: { ...entry[1], receivedBytes: payload.receivedBytes, totalBytes: payload.totalBytes } } : current;
      });
    }).then(keep).catch(() => undefined);
    void refresh();
    return () => {
      live = false;
      stops.forEach((stop) => stop());
    };
  }, []);

  const choose = (codes: string[]) => {
    setRemoved([]);
    setInstallError(null);
    rememberLanguages(codes);
    onChange(codes);
  };

  const describe = (code: string): OcrLanguage =>
    languages.find((language) => language.code === code) ?? (code === BUILT_IN_OCR_LANGUAGE.code ? BUILT_IN_OCR_LANGUAGE : { code, name: code, packId: null, status: "notInstalled", downloadBytes: null });
  const chosen = value.map(describe);
  const installed = languages.filter(usable);
  const downloadable = languages.filter((language) => !usable(language));
  const jobFor = (language: OcrLanguage) => language.packId ? jobs[language.packId] : undefined;

  const install = async (language: OcrLanguage) => {
    const packId = language.packId;
    if (!packId) return;
    const runId = crypto.randomUUID();
    setInstallError(null);
    setJobs((current) => ({ ...current, [packId]: { runId, receivedBytes: 0, totalBytes: 0 } }));
    try {
      await invoke("install_pack", { packId, runId });
    } catch (reason) {
      // A pack error carries its wording; a cancelled install leaves the language as it was.
      const failure = reason as { kind?: string; message?: string } | null;
      if (failure?.kind !== "cancelled") setInstallError(failure?.message || String(reason));
    } finally {
      setJobs((current) => {
        if (current[packId]?.runId !== runId) return current;
        const { [packId]: _finished, ...rest } = current;
        return rest;
      });
    }
  };

  const move = (index: number, to: number) => {
    const next = [...value];
    [next[index], next[to]] = [next[to], next[index]];
    choose(next);
  };

  const option = (language: OcrLanguage) => {
    const added = value.includes(language.code);
    return (
      <SelectItem
        key={language.code}
        value={language.code}
        disabled={added}
        aria-label={added ? `${optionLabel(language)}, already added` : optionLabel(language)}
        aria-labelledby={undefined}
      >
        {language.name}
        {added ? (
          <>
            <Check aria-hidden="true" />
            <span className="text-muted-foreground">Added</span>
          </>
        ) : !usable(language) ? (
          <>
            <Download aria-hidden="true" />
            {language.downloadBytes ? <span className="text-muted-foreground">{formatBytes(language.downloadBytes)}</span> : null}
          </>
        ) : null}
      </SelectItem>
    );
  };

  const notice = removed.length > 0 ? removedNotice(removed) : null;

  return (
    <div className="workspace-field grid gap-2">
      <Label id="ocr-languages-label">Languages, in order of priority</Label>
      {notice && (
        <div className="workspace-note flex items-start gap-2" role="status">
          <TriangleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
          <p><strong>{notice.title}</strong> {notice.body}</p>
        </div>
      )}
      <ol aria-labelledby="ocr-languages-label" className="m-0 grid list-none gap-1 p-0">
        {chosen.map((language, index) => {
          const job = jobFor(language);
          const status = job
            ? `Downloading${job.totalBytes > 0 ? ` ${Math.floor((job.receivedBytes / job.totalBytes) * 100)}%` : ""}`
            : statusText(language);
          return (
            <li key={language.code} aria-label={`${language.name}, ${status.toLowerCase()}, priority ${index + 1}`} className="flex items-center gap-2">
              <span className="workspace-panel-copy">{index + 1}</span>
              <span className="flex-1">{language.name}</span>
              <span className="workspace-panel-copy" role={job ? "status" : undefined}>{status}</span>
              {value.length > 1 && (
                <>
                  {index === 0 ? (
                    <Button variant="ghost" size="icon-sm" aria-label={`Move ${language.name} down`} onClick={() => move(index, index + 1)}>
                      <ArrowDown aria-hidden="true" />
                    </Button>
                  ) : (
                    <Button variant="ghost" size="icon-sm" aria-label={`Move ${language.name} up`} onClick={() => move(index, index - 1)}>
                      <ArrowUp aria-hidden="true" />
                    </Button>
                  )}
                  <Button variant="ghost" size="icon-sm" aria-label={`Remove ${language.name}`} onClick={() => choose(value.filter((code) => code !== language.code))}>
                    <X aria-hidden="true" />
                  </Button>
                </>
              )}
            </li>
          );
        })}
      </ol>
      <div className="grid gap-1">
        <Label htmlFor="ocr-add-language" className="workspace-panel-copy">Add a language</Label>
        <Select value="" onValueChange={(code) => { if (code && !value.includes(code)) choose([...value, code]); }}>
          <SelectTrigger id="ocr-add-language" className="w-full">
            <SelectValue placeholder="Choose…" />
          </SelectTrigger>
          <SelectContent position="popper">
            <SelectGroup>
              <SelectLabel>Installed</SelectLabel>
              {installed.map(option)}
            </SelectGroup>
            {downloadable.length > 0 && (
              <SelectGroup>
                <SelectLabel>Download to use</SelectLabel>
                {downloadable.map(option)}
              </SelectGroup>
            )}
            <p className="workspace-panel-copy px-2 py-1.5">Choosing one adds it to the list. It downloads when you click Install or run.</p>
          </SelectContent>
        </Select>
      </div>
      {chosen.filter((language) => !usable(language) && !jobFor(language)).map((language) => {
        const verb = language.status in packVerbs ? packVerbs[language.status as keyof typeof packVerbs] : undefined;
        const canInstall = language.packId !== null && verb !== undefined && language.downloadBytes !== null;
        return (
          <div key={language.code} className="workspace-note flex items-start gap-2">
            <TriangleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
            <div className="grid gap-2">
              {language.status === "notInstalled" && language.downloadBytes ? (
                <p><strong>{language.name} needs a {formatBytes(language.downloadBytes)} download.</strong> Toolbox downloads it once, then works offline.</p>
              ) : language.status === "damaged" ? (
                <p><strong>{language.name} needs repair.</strong> Toolbox downloads it again, then works offline.</p>
              ) : language.status === "incompatible" ? (
                <p><strong>{language.name} doesn&apos;t work with this version of Toolbox.</strong>{canInstall ? " Toolbox downloads the version that does." : " Update Toolbox to read it."}</p>
              ) : (
                <p><strong>{language.name} isn&apos;t available.</strong> Remove it to run OCR.</p>
              )}
              {canInstall && (
                <Button variant="outline" size="sm" className="justify-self-start" onClick={() => void install(language)}>
                  {verb} {language.name}
                </Button>
              )}
            </div>
          </div>
        );
      })}
      {installError && <p className="workspace-note" role="alert">{installError}</p>}
      <p className="workspace-panel-copy">Recognition runs on this device. Your last choice is remembered.</p>
    </div>
  );
};
