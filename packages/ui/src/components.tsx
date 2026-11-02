// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Components of the Oikome apps (styles in ui.css).

import { useId, useState } from "react";
import type { ButtonHTMLAttributes, ReactNode } from "react";
import { Icon } from "./icons";
import type { IconName } from "./icons";

function cx(...names: (string | false | undefined)[]): string {
  return names.filter(Boolean).join(" ");
}

export type ButtonVariant = "primary" | "secondary" | "text" | "quiet";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** `primary` is the one crimson action of a view. */
  variant?: ButtonVariant;
  large?: boolean;
  icon?: IconName;
}

export function Button({
  variant = "secondary",
  large,
  icon,
  className,
  children,
  type = "button",
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      className={cx("oik-button", `oik-button--${variant}`, large && "oik-button--lg", className)}
      {...rest}
    >
      {icon && <Icon name={icon} />}
      {children}
    </button>
  );
}

export function IconButton({
  icon,
  label,
  ...rest
}: { icon: IconName; label: string } & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button type="button" className="oik-icon-button" aria-label={label} title={label} {...rest}>
      <Icon name={icon} />
    </button>
  );
}

export function Card({ children, className }: { children: ReactNode; className?: string }) {
  return <section className={cx("oik-card", className)}>{children}</section>;
}

export function InsetWell({ children }: { children: ReactNode }) {
  return <div className="oik-inset-well">{children}</div>;
}

/** A grouped inset list: a title, rows on a surface, a footnote. */
export function Group({
  title,
  footnote,
  children,
}: {
  title?: string;
  footnote?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="oik-group">
      {title && <h2 className="oik-group__title">{title}</h2>}
      <ul className="oik-group__body">{children}</ul>
      {footnote && <p className="oik-group__footnote">{footnote}</p>}
    </section>
  );
}

export interface RowProps {
  leading?: ReactNode;
  title: ReactNode;
  meta?: ReactNode;
  trailing?: ReactNode;
  /** Makes the whole row a button. */
  onClick?: () => void;
  selected?: boolean;
  chevron?: boolean;
  add?: boolean;
  /** A message in place of items ("Nothing shared yet"). */
  muted?: boolean;
  label?: string;
}

export function Row({
  leading,
  title,
  meta,
  trailing,
  onClick,
  selected,
  chevron,
  add,
  muted,
  label,
}: RowProps) {
  const body = (
    <>
      {leading}
      <span className="oik-row__text">
        <span className={cx("oik-row__title", muted && "oik-row__title--muted")}>{title}</span>
        {meta && <span className="oik-row__meta">{meta}</span>}
      </span>
      {(trailing || chevron) && (
        <span className="oik-row__trailing">
          {trailing}
          {chevron && (
            <span className="oik-row__chevron">
              <Icon name="chevron" />
            </span>
          )}
        </span>
      )}
    </>
  );
  const className = cx("oik-row", add && "oik-row--add", selected && "oik-row--selected");
  return (
    <li>
      {onClick ? (
        <button
          type="button"
          className={cx(className, "oik-row--button")}
          onClick={onClick}
          aria-current={selected ? "true" : undefined}
          aria-label={label}
        >
          {body}
        </button>
      ) : (
        <div className={className}>{body}</div>
      )}
    </li>
  );
}

export const TILE_TONES = [
  "honey",
  "rose",
  "sage",
  "periwinkle",
  "mist",
  "oat",
  "lavender",
  "stone",
] as const;
export type TileTone = (typeof TILE_TONES)[number] | "apricot";

/** A stable tone for a name, so a collection keeps its color. */
export function toneFor(name: string): TileTone {
  let hash = 0;
  for (const c of name) hash = (hash * 31 + c.charCodeAt(0)) >>> 0;
  return TILE_TONES[hash % TILE_TONES.length] ?? "stone";
}

export function Tile({ icon, tone, large }: { icon: IconName; tone: TileTone; large?: boolean }) {
  return (
    <span className={cx("oik-tile", `oik-tile--${tone}`, large && "oik-tile--lg")} aria-hidden="true">
      <Icon name={icon} />
    </span>
  );
}

export function Avatar({ name, small }: { name: string; small?: boolean }) {
  const tone = toneFor(name);
  return (
    <span
      className={cx("oik-avatar", `oik-tile--${tone}`, small && "oik-avatar--sm")}
      aria-hidden="true"
    >
      {(name.trim()[0] ?? "?").toUpperCase()}
    </span>
  );
}

export function AvatarStack({ names, label }: { names: string[]; label: string }) {
  return (
    <span className="oik-avatar-stack" role="img" aria-label={label}>
      {names.slice(0, 4).map((name) => (
        <Avatar key={name} name={name} small />
      ))}
    </span>
  );
}

interface FieldProps {
  label: string;
  hint?: ReactNode;
  error?: string;
}

export function TextField({
  label,
  hint,
  error,
  value,
  onChange,
  placeholder,
  mono,
  autoFocus,
}: FieldProps & {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  mono?: boolean;
  autoFocus?: boolean;
}) {
  const id = useId();
  return (
    <div className="oik-field">
      <label className="oik-field__label" htmlFor={id}>
        {label}
      </label>
      <input
        id={id}
        className={cx("oik-input", mono && "oik-input--mono")}
        value={value}
        placeholder={placeholder}
        autoFocus={autoFocus}
        spellCheck={mono ? false : undefined}
        aria-invalid={error ? true : undefined}
        onChange={(e) => onChange(e.target.value)}
      />
      {hint && <span className="oik-field__hint">{hint}</span>}
      {error && <ErrorText>{error}</ErrorText>}
    </div>
  );
}

export function TextArea({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  const id = useId();
  return (
    <div className="oik-field">
      <label className="oik-field__label" htmlFor={id}>
        {label}
      </label>
      <textarea
        id={id}
        className="oik-textarea"
        value={value}
        spellCheck={false}
        onChange={(e) => onChange(e.target.value)}
      />
    </div>
  );
}

export function Select({
  label,
  value,
  onChange,
  options,
}: FieldProps & {
  value: string;
  onChange: (value: string) => void;
  options: { value: string; label: string }[];
}) {
  const id = useId();
  return (
    <div className="oik-field">
      <label className="oik-field__label" htmlFor={id}>
        {label}
      </label>
      <select id={id} className="oik-select" value={value} onChange={(e) => onChange(e.target.value)}>
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </div>
  );
}

export function Checkbox({
  label,
  checked,
  onChange,
  disabled,
}: {
  label: ReactNode;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label className="oik-checkbox">
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span>{label}</span>
    </label>
  );
}

export function SegmentedControl<T extends string>({
  label,
  value,
  onChange,
  options,
}: {
  label: string;
  value: T;
  onChange: (value: T) => void;
  options: { value: T; label: string }[];
}) {
  return (
    <div className="oik-field">
      <span className="oik-field__label">{label}</span>
      <div className="oik-segmented" role="group" aria-label={label}>
        {options.map((o) => (
          <button
            key={o.value}
            type="button"
            className="oik-segmented__option"
            aria-pressed={o.value === value}
            onClick={() => onChange(o.value)}
          >
            {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}

/** A warning: amber, with a triangle. */
export function Banner({ title, body, action }: { title: string; body?: string; action?: ReactNode }) {
  return (
    <div className="oik-banner" role="status">
      <Icon name="warning" />
      <span className="oik-banner__text">
        <span className="oik-banner__title">{title}</span>
        {body && <span className="oik-banner__body">{body}</span>}
      </span>
      {action}
    </div>
  );
}

/** An error: text with an icon, saying what happened and what to do. */
export function ErrorText({ children }: { children: ReactNode }) {
  return (
    <p className="oik-error" role="alert">
      <Icon name="error" />
      <span>{children}</span>
    </p>
  );
}

export function EmptyState({
  title,
  body,
  action,
}: {
  title: string;
  body?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="oik-empty">
      <h3 className="oik-empty__title">{title}</h3>
      {body && <p className="oik-empty__body">{body}</p>}
      {action}
    </div>
  );
}

export function StepDots({ step, total, label }: { step: number; total: number; label: string }) {
  return (
    <div className="oik-steps">
      {Array.from({ length: total }, (_, i) => (
        <span
          key={i}
          className={cx("oik-steps__dot", i + 1 === step && "oik-steps__dot--current")}
          aria-hidden="true"
        />
      ))}
      <span>{label}</span>
    </div>
  );
}

/** A value to copy (an ID, an invite code), with a Copy button. */
export function CopyField({
  value,
  copyLabel,
  copiedLabel,
}: {
  value: string;
  copyLabel: string;
  copiedLabel: string;
}) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="oik-copy">
      <p className="oik-copy__value">{value}</p>
      {copied ? (
        <span className="oik-copy__done" role="status">
          {copiedLabel}
        </span>
      ) : (
        <Button
          icon="copy"
          onClick={async () => {
            try {
              await navigator.clipboard.writeText(value);
              setCopied(true);
            } catch {
              // The value stays on screen to copy by hand.
            }
          }}
        >
          {copyLabel}
        </Button>
      )}
    </div>
  );
}
