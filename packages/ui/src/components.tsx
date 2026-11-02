// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Components of the Oikome apps (styles in ui.css).

import { useEffect, useId, useRef, useState } from "react";
import type { ButtonHTMLAttributes, KeyboardEvent, MouseEvent, ReactNode } from "react";
import { Icon } from "./icons";
import type { IconName } from "./icons";

function cx(...names: (string | false | undefined)[]): string {
  return names.filter(Boolean).join(" ");
}

export type ButtonVariant = "primary" | "secondary" | "text" | "quiet" | "destructive";

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

/**
 * A grouped inset list: a title, rows on a surface, a footnote. `list` is
 * for choosing one item (padded, rounded rows, no rules between them).
 */
export function Group({
  title,
  footnote,
  variant,
  children,
}: {
  title?: string;
  footnote?: ReactNode;
  variant?: "list";
  children: ReactNode;
}) {
  return (
    <section className={cx("oik-group", variant && `oik-group--${variant}`)}>
      {title && <h2 className="oik-group__title">{title}</h2>}
      <ul className="oik-group__body">{children}</ul>
      {footnote && <p className="oik-group__footnote">{footnote}</p>}
    </section>
  );
}

/** Free content inside a group: a form, a key, a value to copy. */
export function GroupPanel({ children }: { children: ReactNode }) {
  return (
    <li className="oik-group__item">
      <div className="oik-group__panel">{children}</div>
    </li>
  );
}

export interface RowProps {
  leading?: ReactNode;
  title: ReactNode;
  meta?: ReactNode;
  trailing?: ReactNode;
  /** Makes the whole row a button, or with `href` a link that `onClick` follows. */
  onClick?: () => void;
  /** Makes the whole row a link to another page. */
  href?: string;
  selected?: boolean;
  chevron?: boolean;
  add?: boolean;
  /** A message in place of items ("Nothing shared yet"), or an item that ended. */
  muted?: boolean;
  /** Top-aligns the leading tile, for rows with a long description. */
  alignStart?: boolean;
  label?: string;
}

export function Row({
  leading,
  title,
  meta,
  trailing,
  onClick,
  href,
  selected,
  chevron,
  add,
  muted,
  alignStart,
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
  const className = cx(
    "oik-row",
    !leading && "oik-row--plain",
    add && "oik-row--add",
    selected && "oik-row--selected",
    alignStart && "oik-row--start",
  );
  if (href) {
    // A plain click goes through `onClick` (the app's router); modified
    // clicks open the page in a new tab as links do.
    const follow = (e: MouseEvent<HTMLAnchorElement>) => {
      if (!onClick || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return;
      e.preventDefault();
      onClick();
    };
    return (
      <li className="oik-group__item">
        <a
          href={href}
          className={cx(className, "oik-row--button", "oik-row--link")}
          onClick={follow}
          aria-current={selected ? "page" : undefined}
          aria-label={label}
        >
          {body}
        </a>
      </li>
    );
  }
  return (
    <li className="oik-group__item">
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

export function Tile({
  icon,
  tone,
  large,
  small,
}: {
  icon: IconName;
  tone: TileTone;
  large?: boolean;
  small?: boolean;
}) {
  return (
    <span
      className={cx("oik-tile", `oik-tile--${tone}`, large && "oik-tile--lg", small && "oik-tile--sm")}
      aria-hidden="true"
    >
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
  leading,
  inline,
}: FieldProps & {
  value: string;
  onChange: (value: string) => void;
  options: { value: string; label: string }[];
  /** A tile or avatar for the chosen option, shown inside the control. */
  leading?: ReactNode;
  /** The label beside the control, for a filter in a header. */
  inline?: boolean;
}) {
  const id = useId();
  return (
    <div className={cx("oik-field", inline && "oik-field--inline")}>
      <label className="oik-field__label" htmlFor={id}>
        {label}
      </label>
      <span className={cx("oik-select-wrap", leading !== undefined && "oik-select-wrap--leading")}>
        {leading !== undefined && <span className="oik-select-wrap__leading">{leading}</span>}
        <select id={id} className="oik-select" value={value} onChange={(e) => onChange(e.target.value)}>
          {options.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
        <span className="oik-select-wrap__chevron" aria-hidden="true">
          <Icon name="updown" />
        </span>
      </span>
    </div>
  );
}

export interface Choice<T extends string> {
  value: T;
  label: string;
  description?: string;
  disabled?: boolean;
}

/** A few explained options, one chosen: radio buttons drawn as cards. */
export function ChoiceGroup<T extends string>({
  label,
  value,
  onChange,
  choices,
}: {
  label: string;
  value: T;
  onChange: (value: T) => void;
  choices: Choice<T>[];
}) {
  const name = useId();
  return (
    <fieldset className="oik-choices">
      <legend className="oik-field__label">{label}</legend>
      {choices.map((c) => (
        <label
          key={c.value}
          className={cx(
            "oik-choice",
            c.value === value && "oik-choice--on",
            c.disabled && "oik-choice--disabled",
          )}
        >
          <input
            type="radio"
            name={name}
            value={c.value}
            checked={c.value === value}
            disabled={c.disabled}
            onChange={() => onChange(c.value)}
          />
          <span className="oik-choice__text">
            <span className="oik-choice__label">{c.label}</span>
            {c.description && <span className="oik-choice__description">{c.description}</span>}
          </span>
        </label>
      ))}
    </fieldset>
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
  fill,
}: {
  label: string;
  value: T;
  onChange: (value: T) => void;
  options: { value: T; label: string }[];
  /** Spans the width, options of equal size. */
  fill?: boolean;
}) {
  return (
    <div className="oik-field">
      <span className="oik-field__label">{label}</span>
      <div className={cx("oik-segmented", fill && "oik-segmented--fill")} role="group" aria-label={label}>
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
    <p className="oik-steps">
      {Array.from({ length: total }, (_, i) => (
        <span
          key={i}
          className={cx("oik-steps__dot", i + 1 === step && "oik-steps__dot--current")}
          aria-hidden="true"
        />
      ))}
      <span className="oik-visually-hidden">{label}</span>
    </p>
  );
}

/** Copies a value; says so once it has. */
export function CopyButton({
  value,
  copyLabel,
  copiedLabel,
}: {
  value: string;
  copyLabel: string;
  copiedLabel: string;
}) {
  const [copied, setCopied] = useState(false);
  return copied ? (
    <span className="oik-copy__done" role="status">
      <Icon name="check" />
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
  return (
    <div className="oik-copy">
      <p className="oik-copy__value">{value}</p>
      <CopyButton value={value} copyLabel={copyLabel} copiedLabel={copiedLabel} />
    </div>
  );
}

export interface MenuItem {
  label: string;
  onSelect: () => void;
  /** Removes or ends something: shown in the error color. */
  destructive?: boolean;
}

/**
 * A More (…) button and the actions behind it. Opens from the button,
 * moves with the arrow keys, closes on Escape, Tab or a click elsewhere.
 */
export function Menu({ label, items }: { label: string; items: MenuItem[] }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const listId = useId();

  useEffect(() => {
    if (!open) return;
    root.current?.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus();
    const away = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", away);
    return () => document.removeEventListener("pointerdown", away);
  }, [open]);

  const close = () => {
    setOpen(false);
    trigger.current?.focus();
  };
  const move = (e: KeyboardEvent<HTMLUListElement>) => {
    const all = Array.from(root.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []);
    const at = all.indexOf(document.activeElement as HTMLButtonElement);
    const focus = (i: number) => all[(i + all.length) % all.length]?.focus();
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      focus(at + 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      focus(at - 1);
    } else if (e.key === "Home") {
      e.preventDefault();
      focus(0);
    } else if (e.key === "End") {
      e.preventDefault();
      focus(all.length - 1);
    } else if (e.key === "Tab") {
      setOpen(false);
    }
  };

  return (
    <div className="oik-menu" ref={root}>
      <button
        ref={trigger}
        type="button"
        className="oik-icon-button oik-icon-button--filled"
        aria-label={label}
        title={label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        onClick={() => setOpen((v) => !v)}
      >
        <Icon name="more" />
      </button>
      {open && (
        <ul id={listId} role="menu" aria-label={label} className="oik-menu__list" onKeyDown={move}>
          {items.map((item) => (
            <li key={item.label} role="none">
              <button
                type="button"
                role="menuitem"
                className={cx("oik-menu__item", item.destructive && "oik-menu__item--destructive")}
                onClick={() => {
                  close();
                  item.onSelect();
                }}
              >
                {item.label}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
