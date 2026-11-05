import { useState } from 'react';
import { FolderIcon } from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import type { OutputLocation } from '../contracts';
import { shortenHomePath, useHomeDir } from '../homePath';
import { parentFolder } from './intake';
import type { FileProgress } from './liveRun';
import type { RunDestinationControl } from './ToolScaffold';

const lastFolder = (folder: string) => folder.split(/[\\/]/).filter(Boolean).pop() ?? folder;

const folderFor = (location: OutputLocation, inputPath: string | null) =>
    location === 'alongsideInput' ? (inputPath ? parentFolder(inputPath) : null) : location.customFolder;

/** Where this export goes, next to Export. Changing it here lasts for this export only and leaves Settings alone. */
export const ExportDestination = ({ destination, inputPath }: { destination: RunDestinationControl; inputPath: string | null }) => {
    const home = useHomeDir();
    const [open, setOpen] = useState(false);
    const folder = folderFor(destination.location, inputPath);
    const full = folder ? shortenHomePath(folder, home) : 'Next to the original';
    const short = folder ? lastFolder(folder) : 'Next to the original';
    const defaultLabel = destination.defaultLocation === 'alongsideInput' ? 'Next to the original' : shortenHomePath(destination.defaultLocation.customFolder, home);
    const choose = () => {
        setOpen(false);
        destination.change();
    };
    const useDefault = () => {
        setOpen(false);
        destination.reset();
    };
    return (
        <>
            <Button variant="ghost" size="sm" className="export-destination" aria-haspopup="dialog" aria-expanded={open}
                aria-label={`Save to ${full}${destination.overridden ? ', this export only' : ''}. Change for this export`}
                onClick={() => setOpen(true)}>
                <FolderIcon aria-hidden="true" />
                <span className="export-destination-name">{short}</span>
                {destination.overridden && <Badge variant="secondary">This export only</Badge>}
            </Button>
            <Dialog open={open} onOpenChange={setOpen}>
                <DialogContent className="sm:max-w-[380px]">
                    <DialogHeader>
                        <DialogTitle>Save this export to</DialogTitle>
                        <DialogDescription>For this export only. Your default in Settings stays “{defaultLabel}”.</DialogDescription>
                    </DialogHeader>
                    <p className="export-destination-path" data-testid="export-destination-path">
                        <FolderIcon aria-hidden="true" />
                        <span>{full}</span>
                    </p>
                    <DialogFooter>
                        <Button variant="outline" size="sm" disabled={!destination.overridden} onClick={useDefault}>Use default</Button>
                        <Button variant="outline" size="sm" onClick={choose}>Choose folder…</Button>
                    </DialogFooter>
                </DialogContent>
            </Dialog>
        </>
    );
};

/** The page an export is on, shown in place of Export while it runs. */
export const ExportProgress = ({ step, fileName }: { step: FileProgress | undefined; fileName: string }) => {
    const percent = step ? Math.round((step.completed / step.total) * 100) : undefined;
    const page = step ? Math.min(step.completed + 1, step.total) : null;
    return (
        <span className="export-progress">
            <span role="progressbar" className="export-progress-track" aria-label={`Exporting ${fileName}`}
                aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}>
                <span className="export-progress-fill" style={{ width: `${percent ?? 0}%` }} />
            </span>
            <span className="export-progress-text" role="status">{step && page ? `Page ${page} of ${step.total}` : 'Exporting…'}</span>
        </span>
    );
};
