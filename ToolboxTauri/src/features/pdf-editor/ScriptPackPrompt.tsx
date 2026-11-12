import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { TriangleAlert } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Card, CardContent } from '@/components/ui/card';
import { packVerbs } from '../../components/resultRow';
import { formatBytes, PACK_CHANGED_EVENT, PROGRESS_EVENT, type PackProgress } from '../../views/PacksSection';
import type { ScriptPackNeed, ScriptPackState } from './scriptPacks';

type InstallJob = { runId: string; packId: string; receivedBytes: number; totalBytes: number };

/**
 * The script packs the catalog offers and their state, refreshed whenever a pack is installed or
 * removed, here or in Settings. `changes` counts those refreshes, so a preview that failed for a
 * missing pack can be rendered again.
 */
export const useScriptPacks = () => {
  const [states, setStates] = useState<ScriptPackState[]>([]);
  const [changes, setChanges] = useState(0);
  const [job, setJob] = useState<InstallJob | null>(null);
  const [installError, setInstallError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    const refresh = () => invoke<ScriptPackState[]>('script_packs')
      .then((list) => { if (live) setStates(Array.isArray(list) ? list : []); })
      .catch(() => undefined);
    const stops: UnlistenFn[] = [];
    const keep = (unlisten: UnlistenFn) => { if (live) stops.push(unlisten); else unlisten(); };
    listen(PACK_CHANGED_EVENT, () => { setChanges((value) => value + 1); void refresh(); }).then(keep).catch(() => undefined);
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

  const install = async (packId: string) => {
    const runId = crypto.randomUUID();
    setInstallError(null);
    setJob({ runId, packId, receivedBytes: 0, totalBytes: 0 });
    try {
      await invoke('install_pack', { packId, runId });
    } catch (reason) {
      // A pack error carries its wording; a cancelled install leaves the pack as it was.
      const failure = reason as { kind?: string; message?: string } | null;
      if (failure?.kind !== 'cancelled') setInstallError(failure?.message || String(reason));
    } finally {
      setJob((current) => current?.runId === runId ? null : current);
    }
  };

  return { states, changes, job, installError, install };
};

const tail = { notInstalled: "Until it's installed, this box can't be exported.", damaged: "Until it's repaired, this box can't be exported.", incompatible: "Until it's updated, this box can't be exported." } as const;

/** The text inspector's note for text in a script whose pack isn't usable, from the PdfText board. */
export const ScriptPackPrompt = ({ need, job, installError, onInstall }: {
  need: ScriptPackNeed;
  job: InstallJob | null;
  installError: string | null;
  onInstall: (packId: string) => void;
}) => {
  const { sample, state } = need;
  const status = state.status === 'installed' || state.status === 'updateAvailable' ? 'notInstalled' : state.status;
  const installing = job?.packId === state.packId;
  const canInstall = state.downloadBytes !== null;
  return (
    <Card role="status" className="scene-script-pack gap-0 py-3">
      <CardContent className="scene-script-pack-content px-3">
        <p className="scene-script-pack-message">
          <TriangleAlert aria-hidden="true" />
          <span><strong>“{sample}” can&apos;t be shown yet.</strong> {state.script} needs the {state.packName} pack. {tail[status]}</span>
        </p>
        {canInstall && <div className="scene-script-pack-action">
          <Button variant="outline" size="sm" disabled={installing} onClick={() => onInstall(state.packId)}>{packVerbs[status]} script pack</Button>
          <span className="scene-property-note">{installing
            ? `Downloading${job.totalBytes > 0 ? ` ${Math.floor((job.receivedBytes / job.totalBytes) * 100)}%` : '…'}`
            : `${formatBytes(state.downloadBytes ?? 0)} download`}</span>
        </div>}
        {installError && <p className="scene-property-note" role="alert">{installError}</p>}
      </CardContent>
    </Card>
  );
};
