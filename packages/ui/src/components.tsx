// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Components of the Oikome apps (styles in ui.css).

import { createContext, useCallback, useContext, useEffect, useId, useRef, useState } from "react";
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
  /** Turns its icon while the work it started runs, such as a sync. */
  spinning?: boolean;
}

export function Button({
  variant = "secondary",
  large,
  icon,
  spinning,
  className,
  children,
  type = "button",
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      className={cx(
        "oik-button",
        `oik-button--${variant}`,
        large && "oik-button--lg",
        spinning && "oik-is-spinning",
        className,
      )}
      aria-busy={spinning || undefined}
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
  filled,
  spinning,
  className,
  ...rest
}: {
  icon: IconName;
  label: string;
  filled?: boolean;
  /** Turns its glyph while the work it started runs, such as a sync. */
  spinning?: boolean;
} & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      className={cx("oik-icon-button", filled && "oik-icon-button--filled", spinning && "oik-is-spinning", className)}
      aria-busy={spinning || undefined}
      aria-label={label}
      title={label}
      {...rest}
    >
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
  action,
  footnote,
  variant,
  children,
}: {
  title?: string;
  /** Beside the title in the card's head, such as an Add button. */
  action?: ReactNode;
  footnote?: ReactNode;
  variant?: "list";
  children: ReactNode;
}) {
  return (
    <>
      <section className={cx("oik-group", variant && `oik-group--${variant}`)}>
        {(title || action) && (
          <header className="oik-group__head">
            {title && <h2 className="oik-group__title">{title}</h2>}
            {action}
          </header>
        )}
        <ul className="oik-group__body">{children}</ul>
      </section>
      {footnote && <p className="oik-group__footnote">{footnote}</p>}
    </>
  );
}

export type StatusKind = "ok" | "run" | "idle" | "off" | "bad";

/**
 * A state in a word and a glyph: a filled dot (fine), a turning half ring
 * (working), an empty ring (waiting), a dashed ring (ended), crimson
 * (needs the owner). Bold draws them as colored pills.
 */
export function Status({ kind, children }: { kind: StatusKind; children: ReactNode }) {
  return (
    <span className={`oik-status oik-status--${kind}`}>
      <i aria-hidden="true" />
      {children}
    </span>
  );
}

/** A few options side by side, one of them on. */
export function Segmented({
  label,
  value,
  onChange,
  options,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: { value: string; label: string }[];
}) {
  return (
    <span className="oik-seg" role="radiogroup" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          role="radio"
          aria-checked={option.value === value}
          className="oik-seg__option"
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </span>
  );
}

/**
 * An on/off switch for something that takes effect at once (lending this
 * computer); its knob slides as it changes.
 */
export function Switch({
  label,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      title={label}
      disabled={disabled}
      className={cx("oik-switch", checked && "oik-switch--on")}
      onClick={() => onChange(!checked)}
    />
  );
}

/** A short tag beside a name: "Example", "Default". */
export function Tag({ children }: { children: ReactNode }) {
  return <span className="oik-tag">{children}</span>;
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
  /** A small label above the title: "Name" over the name. */
  overline?: ReactNode;
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
  overline,
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
        {overline && <span className="oik-row__overline">{overline}</span>}
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

export type TileTone =
  | "honey"
  | "rose"
  | "sage"
  | "periwinkle"
  | "mist"
  | "oat"
  | "lavender"
  | "stone"
  | "apricot";

/**
 * Tones a collection can take. Stone means "ended", apricot belongs to the
 * recovery key, and orchid ("rose") sits too close to the crimson of a
 * selection, so none of them names a collection.
 */
const COLLECTION_TONES: TileTone[] = ["honey", "sage", "periwinkle", "oat", "lavender", "mist"];

function hash(name: string): number {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return h;
}

/**
 * A tone for each collection, so neighbours never share one while there are
 * tones left: each name starts at its hash and moves to the next free tone.
 * Screens build it from the same list, so a collection keeps its color.
 */
export function collectionTones(names: string[]): (name: string) => TileTone {
  const tones = new Map<string, TileTone>();
  const used = new Set<TileTone>();
  for (const name of [...names].sort()) {
    let i = hash(name) % COLLECTION_TONES.length;
    for (let tries = 0; tries < COLLECTION_TONES.length && used.has(COLLECTION_TONES[i]!); tries++) {
      i = (i + 1) % COLLECTION_TONES.length;
    }
    const tone = COLLECTION_TONES[i]!;
    tones.set(name, tone);
    used.add(tone);
    if (used.size === COLLECTION_TONES.length) used.clear();
  }
  return (name) => tones.get(name) ?? COLLECTION_TONES[hash(name) % COLLECTION_TONES.length]!;
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

/** A person's initial: "you" in the neutral disc, contacts in orchid or periwinkle. */
export function Avatar({ name, small, you }: { name: string; small?: boolean; you?: boolean }) {
  const tone = you ? "you" : hash(name) % 2 === 0 ? "orchid" : "periwinkle";
  return (
    <span className={cx("oik-avatar", `oik-avatar--${tone}`, small && "oik-avatar--sm")} aria-hidden="true">
      {(name.trim()[0] ?? "?").toUpperCase()}
    </span>
  );
}

/**
 * Who can see something, as a row of small avatars: you first, in the
 * neutral disc, then each contact. The product's signature detail, written
 * the same way wherever it appears.
 */
export function AvatarStack({ names, label }: { names: string[]; label: string }) {
  return (
    <span className="oik-avatar-stack" role="img" aria-label={label}>
      <Avatar name="You" you small />
      {names.slice(0, 3).map((name) => (
        <Avatar key={name} name={name} small />
      ))}
      {names.length > 3 && <span className="oik-avatar-stack__more">+{names.length - 3}</span>}
    </span>
  );
}

/** A row-shaped placeholder while a list loads: a tile and two lines. */
export function SkeletonRows({ count = 3 }: { count?: number }) {
  return (
    <>
      {Array.from({ length: count }, (_, i) => (
        <li key={i} className="oik-group__item" aria-hidden="true">
          <div className="oik-row oik-row--skeleton">
            <span className="oik-skel oik-skel--tile" />
            <span className="oik-row__text">
              <span className="oik-skel oik-skel--line" />
              <span className="oik-skel oik-skel--short" />
            </span>
          </div>
        </li>
      ))}
    </>
  );
}

/* ----- Confirmation toasts: one short line after an action ----- */

const ToastContext = createContext<(message: string) => void>(() => {});

/** Says, briefly, that an action went through ("Removed notes."). */
export function useToast(): (message: string) => void {
  return useContext(ToastContext);
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<{ id: number; message: string }[]>([]);
  const next = useRef(0);
  const show = useCallback((message: string) => {
    const id = next.current++;
    setToasts((list) => [...list.slice(-2), { id, message }]);
    window.setTimeout(() => setToasts((list) => list.filter((t) => t.id !== id)), 3200);
  }, []);
  return (
    <ToastContext.Provider value={show}>
      {children}
      <div className="oik-toasts" role="status" aria-live="polite">
        {toasts.map((t) => (
          <p key={t.id} className="oik-toast">
            <Icon name="check" />
            {t.message}
          </p>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

interface FieldProps {
  label: string;
  hint?: ReactNode;
  error?: string;
}

export function TextField({
  label,
  name,
  hint,
  error,
  value,
  onChange,
  placeholder,
  mono,
  autoFocus,
  selectOnFocus,
  type = "text",
}: FieldProps & {
  /** The field's name; fields here are never credentials, so autofill is off. */
  name: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  mono?: boolean;
  autoFocus?: boolean;
  /** Select a suggested value on focus, so typing replaces it. */
  selectOnFocus?: boolean;
  /** `password` hides what is typed (and lets password managers fill it). */
  type?: "text" | "password" | "url";
}) {
  const id = useId();
  return (
    <div className="oik-field">
      <label className="oik-field__label" htmlFor={id}>
        {label}
      </label>
      <input
        id={id}
        name={name}
        type={type}
        autoComplete={type === "password" ? "current-password" : "off"}
        className={cx("oik-input", mono && "oik-input--mono")}
        value={value}
        placeholder={placeholder}
        autoFocus={autoFocus}
        onFocus={selectOnFocus ? (e) => e.currentTarget.select() : undefined}
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
  name = "text",
  hint,
  error,
  value,
  onChange,
  mono,
  autoFocus,
}: FieldProps & {
  name?: string;
  value: string;
  onChange: (value: string) => void;
  /** Codes and other text read by machines: monospace, no spellcheck. */
  mono?: boolean;
  autoFocus?: boolean;
}) {
  const id = useId();
  return (
    <div className="oik-field">
      <label className="oik-field__label" htmlFor={id}>
        {label}
      </label>
      <textarea
        id={id}
        name={name}
        autoComplete="off"
        className={cx("oik-textarea", mono && "oik-input--mono")}
        value={value}
        spellCheck={false}
        autoFocus={autoFocus}
        aria-invalid={error ? true : undefined}
        onChange={(e) => onChange(e.target.value)}
      />
      {hint && <span className="oik-field__hint">{hint}</span>}
      {error && <ErrorText>{error}</ErrorText>}
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
  options: { value: string; label: string; disabled?: boolean }[];
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
            <option key={o.value} value={o.value} disabled={o.disabled}>
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

export interface Choice {
  value: string;
  title: ReactNode;
  meta?: ReactNode;
  leading?: ReactNode;
  /** Beside the choice, such as a Change… button. */
  aside?: ReactNode;
  /** Shown under the choice while it is chosen: the fields it needs. */
  more?: ReactNode;
}

/** One of a few choices, as cards with a radio each; the chosen one in crimson. */
export function ChoiceList({
  label,
  name,
  value,
  onChange,
  choices,
}: {
  label: string;
  name: string;
  value: string;
  onChange: (value: string) => void;
  choices: Choice[];
}) {
  return (
    <fieldset className="oik-choices">
      <legend className="oik-field__label">{label}</legend>
      {choices.map((choice) => {
        const on = choice.value === value;
        return (
          <div key={choice.value} className={cx("oik-choice", on && "oik-choice--on")}>
            <label className="oik-choice__row">
              <input
                type="radio"
                name={name}
                value={choice.value}
                checked={on}
                onChange={() => onChange(choice.value)}
              />
              {choice.leading && <span className="oik-tile oik-choice__tile">{choice.leading}</span>}
              <span className="oik-choice__text">
                <span className="oik-choice__title">{choice.title}</span>
                {choice.meta && <span className="oik-choice__meta">{choice.meta}</span>}
              </span>
            </label>
            {choice.aside && <span className="oik-choice__aside">{choice.aside}</span>}
            {on && choice.more && <div className="oik-choice__more">{choice.more}</div>}
          </div>
        );
      })}
    </fieldset>
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
  icon,
}: {
  title: string;
  body?: ReactNode;
  action?: ReactNode;
  icon?: IconName;
}) {
  return (
    <div className="oik-empty">
      {icon && <Tile icon={icon} tone="stone" large />}
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
  iconOnly = false,
}: {
  value: string;
  copyLabel: string;
  copiedLabel: string;
  /** Just the icon, for a row whose overline already says what is copied. */
  iconOnly?: boolean;
}) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
    } catch {
      // The value stays on screen to copy by hand.
    }
  };
  if (copied) {
    return (
      <span className="oik-copy__done" role="status">
        <Icon name="check" />
        {iconOnly ? <span className="oik-visually-hidden">{copiedLabel}</span> : copiedLabel}
      </span>
    );
  }
  return iconOnly ? (
    <IconButton icon="copy" label={copyLabel} onClick={copy} />
  ) : (
    <Button icon="copy" onClick={copy}>
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
      <p className="oik-copy__value" translate="no">
        {value}
      </p>
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

/**
 * A modal over the page: the standard place for a short task (sharing) or a
 * question (removing). A native `<dialog>`: the page behind is inert, focus
 * stays inside, Escape closes it. On a phone it rises from the bottom.
 */
export function Dialog({
  title,
  closeLabel,
  onClose,
  footer,
  children,
  closeButton = true,
}: {
  title: string;
  closeLabel: string;
  onClose: () => void;
  footer: ReactNode;
  children?: ReactNode;
  /** A close button in the corner; a question leaves it out (Cancel says it). */
  closeButton?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
    return () => dialog?.close();
  }, []);
  return (
    <dialog
      ref={ref}
      className="oik-dialog"
      aria-labelledby={titleId}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
    >
      <header className="oik-dialog__head">
        <h2 id={titleId} className="oik-dialog__title">
          {title}
        </h2>
        {closeButton && <IconButton icon="close" label={closeLabel} filled onClick={onClose} />}
      </header>
      {children && <div className="oik-dialog__body">{children}</div>}
      <footer className="oik-dialog__actions">{footer}</footer>
    </dialog>
  );
}

/** Asks before removing something; focus starts on Cancel, the safe choice. */
export function ConfirmDialog({
  title,
  body,
  confirmLabel,
  cancelLabel,
  busy,
  error,
  onConfirm,
  onCancel,
}: {
  title: string;
  body: ReactNode;
  confirmLabel: string;
  cancelLabel: string;
  busy?: boolean;
  error?: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <Dialog
      title={title}
      closeLabel={cancelLabel}
      onClose={onCancel}
      closeButton={false}
      footer={
        <>
          <Button autoFocus onClick={onCancel}>
            {cancelLabel}
          </Button>
          <Button variant="destructive" disabled={busy} onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </>
      }
    >
      <p className="oik-dialog__text">{body}</p>
      {error && <ErrorText>{error}</ErrorText>}
    </Dialog>
  );
}
