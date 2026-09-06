import lockOpen from '../assets/tabler/lock-open.svg?url';
import lock from '../assets/tabler/lock.svg?url';
import fileDownload from '../assets/tabler/file-download.svg?url';
import arrowsExchange from '../assets/tabler/arrows-exchange.svg?url';
import briefcase from '../assets/tabler/briefcase.svg?url';

const designIcons = import.meta.glob('../assets/design-icons/*.svg', {
    eager: true,
    import: 'default',
    query: '?url',
}) as Record<string, string>;

const icons = {
    'lock-open': lockOpen,
    lock,
    'file-download': fileDownload,
    'arrows-exchange': arrowsExchange,
    briefcase,
} as const;

type TablerIconName = keyof typeof icons;

export const TablerIcon = ({
    name,
    color = 'currentColor',
    className = '',
}: {
    name: string;
    color?: string;
    className?: string;
}) => {
    const designSource = designIcons[`../assets/design-icons/${name}.svg`];
    const source = designSource ?? icons[name as TablerIconName] ?? briefcase;
    const mask = `url("${source}")`;

    return (
        <span
            aria-hidden="true"
            className={`inline-block shrink-0 bg-current ${className}`}
            style={{
                backgroundColor: color,
                maskImage: mask,
                WebkitMaskImage: mask,
                maskPosition: 'center',
                WebkitMaskPosition: 'center',
                maskRepeat: 'no-repeat',
                WebkitMaskRepeat: 'no-repeat',
                maskSize: 'contain',
                WebkitMaskSize: 'contain',
            }}
        />
    );
};
