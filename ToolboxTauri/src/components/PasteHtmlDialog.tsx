import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';
import { pastedHtmlTitle, pastedSizeNote } from './pastedHtml';

const PasteForm = ({ onAdd, onCancel }: { onAdd: (name: string, markup: string) => void; onCancel: () => void }) => {
    const [markup, setMarkup] = useState('');
    const [name, setName] = useState('');
    const [named, setNamed] = useState(false);
    const shownName = named ? name : pastedHtmlTitle(markup);
    const ready = markup.trim().length > 0 && shownName.trim().length > 0;

    return (
        <form
            className="grid gap-4"
            onSubmit={(event) => {
                event.preventDefault();
                if (ready) onAdd(shownName, markup);
            }}
        >
            <DialogHeader>
                <DialogTitle>Paste HTML</DialogTitle>
                <DialogDescription>The markup joins the list as one item and becomes its own PDF. Images and fonts from the internet aren't loaded.</DialogDescription>
            </DialogHeader>
            <div className="grid gap-2">
                <Label htmlFor="paste-html-name">Name in the list</Label>
                <Input
                    id="paste-html-name"
                    value={shownName}
                    spellCheck={false}
                    aria-describedby="paste-html-name-hint"
                    onChange={(event) => {
                        setNamed(true);
                        setName(event.target.value);
                    }}
                />
                <p id="paste-html-name-hint" className="workspace-note">Taken from the page title. It's also the suggested file name.</p>
            </div>
            <div className="grid gap-2">
                <Label htmlFor="paste-html-markup">Markup</Label>
                <Textarea
                    id="paste-html-markup"
                    value={markup}
                    autoFocus
                    spellCheck={false}
                    rows={8}
                    className="max-h-72 font-mono"
                    aria-describedby="paste-html-markup-hint"
                    onChange={(event) => setMarkup(event.target.value)}
                />
                <p id="paste-html-markup-hint" className="workspace-note">{markup.trim() ? pastedSizeNote(markup) : 'Paste a whole page or a fragment.'}</p>
            </div>
            <DialogFooter>
                <Button type="button" variant="outline" onClick={onCancel}>Cancel</Button>
                <Button type="submit" disabled={!ready}>Add to list</Button>
            </DialogFooter>
        </form>
    );
};

export const PasteHtmlDialog = ({ open, onAdd, onClose }: { open: boolean; onAdd: (name: string, markup: string) => void; onClose: () => void }) => (
    <Dialog open={open} onOpenChange={(isOpen) => { if (!isOpen) onClose(); }}>
        {open && (
            <DialogContent showCloseButton={false} className="sm:max-w-[540px]">
                <PasteForm onAdd={onAdd} onCancel={onClose} />
            </DialogContent>
        )}
    </Dialog>
);
