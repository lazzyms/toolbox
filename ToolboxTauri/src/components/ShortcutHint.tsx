import { shortcutLabel, type ShortcutId } from '../shortcuts';

export const ShortcutHint = ({ id }: { id: ShortcutId }) => (
    <kbd aria-hidden="true" className="shortcut-hint" data-shortcut-hint={id}>{shortcutLabel(id)}</kbd>
);
