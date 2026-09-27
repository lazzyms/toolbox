import {
  forwardRef,
  useId,
  useRef,
  type ButtonHTMLAttributes,
  type CSSProperties,
  type InputHTMLAttributes,
  type KeyboardEvent,
  type ReactNode,
  type SelectHTMLAttributes,
} from "react";

type ButtonVariant = "primary" | "ghost" | "danger";
type ControlSize = "sm" | "md" | "lg";

const classes = (...values: Array<string | undefined | false>) =>
  values.filter(Boolean).join(" ");

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ControlSize;
}

export function Button({
  className,
  size = "md",
  type = "button",
  variant = "ghost",
  ...props
}: ButtonProps) {
  return (
    <button
      {...props}
      className={classes("ds-button", `ds-button--${variant}`, `ds-button--${size}`, className)}
      data-size={size}
      data-variant={variant}
      type={type}
    />
  );
}

export interface IconButtonProps
  extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "aria-label"> {
  "aria-label": string;
  pressed?: boolean;
  size?: ControlSize;
  tone?: "neutral" | "danger";
}

export function IconButton({
  className,
  pressed,
  size = "md",
  tone = "neutral",
  type = "button",
  ...props
}: IconButtonProps) {
  return (
    <button
      {...props}
      aria-pressed={pressed}
      className={classes("ds-icon-button", `ds-icon-button--${tone}`, `ds-icon-button--${size}`, className)}
      data-size={size}
      type={type}
    />
  );
}

export interface SliderProps {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  disabled?: boolean;
  className?: string;
  formatValue?: (value: number) => string;
  onChange: (value: number) => void;
}

export function Slider({
  className,
  disabled = false,
  formatValue = (value) => String(value),
  label,
  max,
  min,
  onChange,
  step = 1,
  value,
}: SliderProps) {
  const id = useId();
  return (
    <div className={classes("ds-slider", className)} data-disabled={disabled || undefined}>
      <div className="ds-slider__heading">
        <label htmlFor={id}>{label}</label>
        <output htmlFor={id}>{formatValue(value)}</output>
      </div>
      <input
        aria-label={label}
        disabled={disabled}
        id={id}
        max={max}
        min={min}
        onChange={(event) => onChange(event.currentTarget.valueAsNumber)}
        step={step}
        type="range"
        value={value}
      />
    </div>
  );
}

export interface ColorSwatchOption {
  label: string;
  value: string;
  disabled?: boolean;
}

export interface ColorSwatchesProps {
  label: string;
  value: string;
  options: readonly ColorSwatchOption[];
  disabled?: boolean;
  className?: string;
  onChange: (value: string) => void;
}

export function ColorSwatches({
  className,
  disabled = false,
  label,
  onChange,
  options,
  value,
}: ColorSwatchesProps) {
  const id = useId();
  return (
    <fieldset className={classes("ds-color-swatches", className)} disabled={disabled}>
      <legend id={`${id}-label`}>{label}</legend>
      <div aria-label={label} className="ds-color-swatches__options" role="group">
        {options.map((option) => (
          <button
            aria-label={option.label}
            aria-pressed={option.value === value}
            className="ds-color-swatch"
            disabled={option.disabled}
            key={option.value}
            onClick={() => onChange(option.value)}
            style={{ "--ds-swatch-color": option.value } as CSSProperties}
            title={option.label}
            type="button"
          >
            <span aria-hidden="true" />
          </button>
        ))}
      </div>
    </fieldset>
  );
}

export interface ToggleProps {
  label: string;
  checked: boolean;
  disabled?: boolean;
  className?: string;
  onChange: (checked: boolean) => void;
}

export function Toggle({
  checked,
  className,
  disabled = false,
  label,
  onChange,
}: ToggleProps) {
  return (
    <button
      aria-checked={checked}
      className={classes("ds-toggle", className)}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      role="switch"
      type="button"
    >
      <span className="ds-toggle__label">{label}</span>
      <span aria-hidden="true" className="ds-toggle__track">
        <span className="ds-toggle__thumb" />
      </span>
    </button>
  );
}

export interface SegmentedOption {
  label: string;
  value: string;
  disabled?: boolean;
}

export interface SegmentedControlProps {
  label: string;
  value: string;
  options: readonly SegmentedOption[];
  disabled?: boolean;
  className?: string;
  onChange: (value: string) => void;
}

export function SegmentedControl({
  className,
  disabled = false,
  label,
  onChange,
  options,
  value,
}: SegmentedControlProps) {
  const buttonRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const handleKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const direction =
      event.key === "ArrowRight" || event.key === "ArrowDown"
        ? 1
        : event.key === "ArrowLeft" || event.key === "ArrowUp"
          ? -1
          : 0;
    const enabledIndexes = options.flatMap((option, optionIndex) =>
      option.disabled ? [] : [optionIndex],
    );
    if (!direction || disabled || enabledIndexes.length === 0) return;
    event.preventDefault();
    const currentPosition = enabledIndexes.indexOf(index);
    const basePosition = currentPosition < 0 ? 0 : currentPosition;
    const nextPosition =
      (basePosition + direction + enabledIndexes.length) % enabledIndexes.length;
    const nextIndex = enabledIndexes[nextPosition];
    buttonRefs.current[nextIndex]?.focus();
    onChange(options[nextIndex].value);
  };

  return (
    <div
      aria-label={label}
      className={classes("ds-segmented-control", className)}
      data-disabled={disabled || undefined}
      role="radiogroup"
    >
      {options.map((option, index) => (
        <button
          aria-checked={option.value === value}
          className="ds-segmented-control__option"
          disabled={disabled || option.disabled}
          key={option.value}
          onClick={() => onChange(option.value)}
          onKeyDown={(event) => handleKeyDown(event, index)}
          ref={(node) => { buttonRefs.current[index] = node; }}
          role="radio"
          tabIndex={option.value === value ? 0 : -1}
          type="button"
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

interface FieldDescriptionProps {
  label: string;
  labelHidden?: boolean;
  hint?: string;
  error?: string;
  className?: string;
  controlClassName?: string;
  leading?: ReactNode;
  trailing?: ReactNode;
}

function FieldDescription({
  error,
  hint,
  id,
}: {
  error?: string;
  hint?: string;
  id: string;
}) {
  return (
    <>
      {hint && <span className="ds-field__hint" id={`${id}-hint`}>{hint}</span>}
      {error && <span className="ds-field__error" id={`${id}-error`} role="alert">{error}</span>}
    </>
  );
}

export interface TextFieldProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, "size">,
    FieldDescriptionProps {
  inputClassName?: string;
}

export const TextField = forwardRef<HTMLInputElement, TextFieldProps>(function TextField(
  {
    "aria-describedby": ariaDescribedBy,
    className,
    controlClassName,
    error,
    hint,
    id: suppliedId,
    inputClassName,
    label,
    labelHidden = false,
    leading,
    trailing,
    type = "text",
    ...props
  },
  ref,
) {
  const generatedId = useId();
  const id = suppliedId ?? generatedId;
  const describedBy = [
    ariaDescribedBy,
    hint && `${id}-hint`,
    error && `${id}-error`,
  ].filter(Boolean).join(" ") || undefined;

  return (
    <label className={classes("ds-field", className)} htmlFor={id}>
      <span className={labelHidden ? "ds-sr-only" : "ds-field__label"}>{label}</span>
      <span className={classes("ds-field__control", controlClassName)}>
        {leading && <span className="ds-field__adornment">{leading}</span>}
        <input
          {...props}
          aria-describedby={describedBy}
          aria-invalid={error ? true : props["aria-invalid"]}
          className={classes("ds-input", inputClassName)}
          id={id}
          ref={ref}
          type={type}
        />
        {trailing && <span className="ds-field__adornment">{trailing}</span>}
      </span>
      <FieldDescription error={error} hint={hint} id={id} />
    </label>
  );
});

export interface NumberFieldProps extends Omit<TextFieldProps, "type"> {}

export const NumberField = forwardRef<HTMLInputElement, NumberFieldProps>(function NumberField(
  props,
  ref,
) {
  return <TextField {...props} ref={ref} type="number" />;
});

export interface SelectOption {
  label: string;
  value: string;
  disabled?: boolean;
}

export interface SelectProps
  extends Omit<SelectHTMLAttributes<HTMLSelectElement>, "size">,
    Omit<FieldDescriptionProps, "leading" | "trailing" | "controlClassName"> {
  options: readonly SelectOption[];
  selectClassName?: string;
}

export const Select = forwardRef<HTMLSelectElement, SelectProps>(function Select(
  {
    "aria-describedby": ariaDescribedBy,
    className,
    error,
    hint,
    id: suppliedId,
    label,
    labelHidden = false,
    options,
    selectClassName,
    ...props
  },
  ref,
) {
  const generatedId = useId();
  const id = suppliedId ?? generatedId;
  const describedBy = [
    ariaDescribedBy,
    hint && `${id}-hint`,
    error && `${id}-error`,
  ].filter(Boolean).join(" ") || undefined;
  return (
    <label className={classes("ds-field", className)} htmlFor={id}>
      <span className={labelHidden ? "ds-sr-only" : "ds-field__label"}>{label}</span>
      <span className="ds-select__control">
        <select
          {...props}
          aria-describedby={describedBy}
          aria-invalid={error ? true : props["aria-invalid"]}
          className={classes("ds-select", selectClassName)}
          id={id}
          ref={ref}
        >
          {options.map((option) => (
            <option disabled={option.disabled} key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      </span>
      <FieldDescription error={error} hint={hint} id={id} />
    </label>
  );
});
