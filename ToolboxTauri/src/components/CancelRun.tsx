import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import type { LiveRun } from './liveRun';
import { ShortcutHint } from './ShortcutHint';
import { matchShortcut, shortcutAria } from '../shortcuts';

export interface RunTally {
    finished: number;
    running: number;
    waiting: number;
}

export const tallyRun = (run: LiveRun): RunTally => {
    const finished = run.finished.filter((outcome) => outcome !== undefined).length;
    const running = run.running.size;
    return { finished, running, waiting: run.paths.length - finished - running };
};

export const cancelConsequences = ({ finished, running, waiting }: RunTally): string => [
    finished === 1 ? 'The 1 finished file keeps its output.' : finished > 1 ? `The ${finished} finished files keep their outputs.` : null,
    running === 1 ? 'The file in progress is stopped and leaves nothing half-written.' : running > 1 ? `The ${running} files in progress are stopped and leave nothing half-written.` : null,
    waiting === 1 ? "The other file won't be processed." : waiting > 1 ? `The other ${waiting} files won't be processed.` : null,
].filter((sentence): sentence is string => sentence !== null).join(' ');

export const CancelRun = ({ run }: { run: LiveRun }) => {
    const [confirming, setConfirming] = useState(false);
    const [requested, setRequested] = useState(false);
    const tally = tallyRun(run);

    useEffect(() => {
        if (requested) return;
        const onKeyDown = (event: KeyboardEvent) => {
            if (confirming || matchShortcut(event, 'window') !== 'cancel') return;
            event.preventDefault();
            setConfirming(true);
        };
        window.addEventListener('keydown', onKeyDown);
        return () => window.removeEventListener('keydown', onKeyDown);
    }, [confirming, requested]);

    const cancel = () => {
        setConfirming(false);
        setRequested(true);
        // The batch ends on its own when the native side stops, so a lost request only means it runs to the end.
        void invoke<boolean>('cancel_run', { runId: run.runId }).catch(() => undefined);
    };

    return (
        <div className="run-controls" role="group" aria-label="Running batch">
            <p className="run-controls-lock">Options are locked until the batch finishes.</p>
            <div className="run-controls-footer">
                <span className="run-controls-hint">{`${tally.finished} of ${run.paths.length} done`}</span>
                <Button variant="outline" size="sm" disabled={requested} aria-keyshortcuts={shortcutAria('cancel')} onClick={() => setConfirming(true)}>
                    Cancel batch
                    <ShortcutHint id="cancel" />
                </Button>
            </div>
            <Dialog open={confirming} onOpenChange={setConfirming}>
                <DialogContent role="alertdialog" showCloseButton={false} className="sm:max-w-[440px]">
                    <DialogHeader>
                        <DialogTitle>Cancel this batch?</DialogTitle>
                        <DialogDescription>{cancelConsequences(tally)}</DialogDescription>
                    </DialogHeader>
                    <DialogFooter>
                        <Button variant="outline" onClick={() => setConfirming(false)}>Keep running</Button>
                        <Button onClick={cancel}>Cancel batch</Button>
                    </DialogFooter>
                </DialogContent>
            </Dialog>
        </div>
    );
};
