import {
  cloneElement,
  createElement,
  isValidElement,
  useEffect,
  useId,
  useRef,
  type ElementType,
  type FocusEvent as ReactFocusEvent,
  type HTMLAttributes,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactElement,
  type ReactNode,
} from "react";
import { IconButton, type IconButtonProps } from "./controls";

const classes = (...values: Array<string | undefined | false>) =>
  values.filter(Boolean).join(" ");

export type StatusTone = "neutral" | "info" | "success" | "warning" | "danger";

export interface PillProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: StatusTone;
}

export function Pill({ className, tone = "neutral", ...props }: PillProps) {
  return <span {...props} className={classes("ds-pill", className)} data-tone={tone} />;
}

export interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: StatusTone;
}

export function Badge({ className, tone = "neutral", ...props }: BadgeProps) {
  return <span {...props} className={classes("ds-badge", className)} data-tone={tone} />;
}

export interface CardProps extends HTMLAttributes<HTMLElement> {
  as?: "article" | "div" | "section";
  interactive?: boolean;
}

export function Card({ as = "div", className, interactive = false, ...props }: CardProps) {
  return createElement(as as ElementType, {
    ...props,
    className: classes("ds-card", interactive && "ds-card--interactive", className),
  });
}

interface HeadingSurfaceProps {
  title: string;
  description?: string;
  actions?: ReactNode;
  children?: ReactNode;
  className?: string;
}

export function Panel({ actions, children, className, description, title }: HeadingSurfaceProps) {
  const titleId = useId();
  return (
    <section aria-labelledby={titleId} className={classes("ds-panel", className)}>
      <header className="ds-panel__header">
        <div>
          <h2 id={titleId}>{title}</h2>
          {description && <p>{description}</p>}
        </div>
        {actions}
      </header>
      {children && <div className="ds-panel__body">{children}</div>}
    </section>
  );
}

export function Section({ actions, children, className, description, title }: HeadingSurfaceProps) {
  const titleId = useId();
  return (
    <section aria-labelledby={titleId} className={classes("ds-section", className)}>
      <header className="ds-section__header">
        <div>
          <h2 id={titleId}>{title}</h2>
          {description && <p>{description}</p>}
        </div>
        {actions}
      </header>
      {children && <div className="ds-section__body">{children}</div>}
    </section>
  );
}

export interface ThumbnailProps extends HTMLAttributes<HTMLElement> {
  src: string;
  alt: string;
  caption?: string;
  selected?: boolean;
}

export function Thumbnail({
  alt,
  caption,
  className,
  selected = false,
  src,
  ...props
}: ThumbnailProps) {
  return (
    <figure
      {...props}
      className={classes("ds-thumbnail", className)}
      data-selected={selected || undefined}
    >
      <img alt={alt} loading="lazy" src={src} />
      {caption && <figcaption>{caption}</figcaption>}
    </figure>
  );
}

export interface TooltipProps {
  content: string;
  children: ReactNode;
  className?: string;
}

export function Tooltip({ children, className, content }: TooltipProps) {
  const id = useId();
  const child = isValidElement<{ "aria-describedby"?: string }>(children)
    ? children
    : null;
  const describedBy = [child?.props["aria-describedby"], id].filter(Boolean).join(" ");
  const trigger = child
    ? cloneElement(child as ReactElement, { "aria-describedby": describedBy })
    : <span aria-describedby={id} tabIndex={0}>{children}</span>;

  return (
    <span className={classes("ds-tooltip", className)}>
      {trigger}
      <span className="ds-tooltip__content" id={id} role="tooltip">{content}</span>
    </span>
  );
}

export interface EmptyStateProps {
  title: string;
  description?: string;
  action?: ReactNode;
  icon?: ReactNode;
  className?: string;
}

export function EmptyState({ action, className, description, icon, title }: EmptyStateProps) {
  return (
    <div className={classes("ds-empty-state", className)}>
      {icon && <span aria-hidden="true" className="ds-empty-state__icon">{icon}</span>}
      <strong>{title}</strong>
      {description && <span>{description}</span>}
      {action && <div className="ds-empty-state__action">{action}</div>}
    </div>
  );
}

export interface ToolbarProps extends HTMLAttributes<HTMLDivElement> {
  label: string;
  orientation?: "horizontal" | "vertical";
}

export function Toolbar({
  children,
  className,
  label,
  orientation = "horizontal",
  onFocusCapture,
  onKeyDown,
  ...props
}: ToolbarProps) {
  const toolbarRef = useRef<HTMLDivElement>(null);
  const enabledButtons = () =>
    Array.from(toolbarRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
  const setTabStop = (target?: HTMLButtonElement) => {
    const buttons = enabledButtons();
    const targetIndex = target
      ? buttons.indexOf(target)
      : buttons.indexOf(document.activeElement as HTMLButtonElement);
    const tabStopIndex = targetIndex >= 0 ? targetIndex : 0;
    buttons.forEach((button, index) => {
      button.tabIndex = index === tabStopIndex ? 0 : -1;
    });
  };

  useEffect(() => {
    setTabStop();
  }, [children]);

  const handleFocusCapture = (event: ReactFocusEvent<HTMLDivElement>) => {
    onFocusCapture?.(event);
    if (!event.defaultPrevented && event.target instanceof HTMLButtonElement && !event.target.disabled) {
      setTabStop(event.target);
    }
  };

  const handleKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    onKeyDown?.(event);
    if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey) return;

    const buttons = enabledButtons();
    const currentIndex = buttons.indexOf(document.activeElement as HTMLButtonElement);
    if (currentIndex < 0 || buttons.length === 0) return;

    const forwardKey = orientation === "horizontal" ? "ArrowRight" : "ArrowDown";
    const backwardKey = orientation === "horizontal" ? "ArrowLeft" : "ArrowUp";
    let nextIndex = -1;
    if (event.key === forwardKey) nextIndex = (currentIndex + 1) % buttons.length;
    if (event.key === backwardKey) nextIndex = (currentIndex - 1 + buttons.length) % buttons.length;
    if (event.key === "Home") nextIndex = 0;
    if (event.key === "End") nextIndex = buttons.length - 1;
    if (nextIndex < 0) return;

    event.preventDefault();
    setTabStop(buttons[nextIndex]);
    buttons[nextIndex].focus();
  };

  return (
    <div
      {...props}
      aria-label={label}
      aria-orientation={orientation}
      className={classes("ds-toolbar", className)}
      onFocusCapture={handleFocusCapture}
      onKeyDown={handleKeyDown}
      ref={toolbarRef}
      role="toolbar"
    >
      {children}
    </div>
  );
}

export type ToolbarButtonProps = IconButtonProps;

export function ToolbarButton({ className, ...props }: ToolbarButtonProps) {
  return <IconButton {...props} className={classes("ds-toolbar-button", className)} size="sm" />;
}

export interface StatusBarProps extends HTMLAttributes<HTMLElement> {}

export function StatusBar({ className, ...props }: StatusBarProps) {
  return <footer {...props} className={classes("ds-status-bar", className)} role="status" />;
}
