// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Home's cards on a twelve-column grid, and Arrange, which turns Home into
// its own editor as the prototype does: each card shows a dashed edge and
// moves where it is dragged (or with the arrow keys), takes a width from
// its corner, and comes off with its minus button; Add a Card puts more on.
// A drag follows the pointer from where the card was taken; the cards it
// passes slide to their new places from wherever they are on screen.

import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import type { CSSProperties, KeyboardEvent, PointerEvent, ReactNode } from "react";
import { Button, Dialog, Icon, IconButton, Tag, Tile } from "@oikome/ui";
import type { IconName } from "@oikome/ui";
import { EASE, reducedMotion } from "./motion";
import { Link } from "./router";
import { strings } from "./strings";

const s = strings.home;

/** Home's columns, and the fewest a card may span. */
const COLUMNS = 12;
const NARROWEST = 4;
/** A mouse moves this far before a card follows it… */
const LIFT_PX = 6;
/** …and a finger holds still this long, so a swipe still scrolls. */
const HOLD_MS = 250;
const HOLD_SLOP_PX = 8;
/** Near the window's top or bottom edge, a drag scrolls the page. */
const EDGE_PX = 64;

/** A card on Home and the columns it spans. */
export interface Placed {
  id: string;
  width: number;
}

/**
 * Widths for Home's cards. A card kept with a width has it; one kept
 * without (Home's first arrangement, or one kept before cards had widths)
 * takes its own and, last in its row, stretches to the row's end, so rows
 * fill as the prototype's do.
 */
export function settle(cards: { id: string; width: number | null; own: number }[]): Placed[] {
  const out: (Placed & { free: boolean })[] = [];
  let used = 0;
  const fill = () => {
    const last = out[out.length - 1];
    if (last?.free && used < COLUMNS) last.width += COLUMNS - used;
  };
  for (const card of cards) {
    const width = card.width ?? card.own;
    if (used > 0 && used + width > COLUMNS) {
      fill();
      used = 0;
    }
    out.push({ id: card.id, width, free: card.width === null });
    used += width;
  }
  fill();
  return out.map(({ id, width }) => ({ id, width }));
}

/** A card Home can show: its id, what it is called, and its first width. */
export interface CardChoice {
  id: string;
  label: string;
  icon: IconName;
  width: number;
}

interface Drag {
  id: string;
  pointerId: number;
  touch: boolean;
  startX: number;
  startY: number;
  /** Where in the card it was taken, so it stays under the pointer there. */
  grabX: number;
  grabY: number;
  x: number;
  y: number;
  live: boolean;
  hold?: number;
  /** The card it last swapped with, until the pointer leaves that card. */
  passed?: string;
}

interface Resize {
  id: string;
  pointerId: number;
  left: number;
}

function same(a: Placed[], b: Placed[]): boolean {
  return a.length === b.length && a.every((card, i) => card.id === b[i]!.id && card.width === b[i]!.width);
}

function moved(list: Placed[], from: number, to: number): Placed[] {
  const next = [...list];
  const [card] = next.splice(from, 1);
  next.splice(to, 0, card!);
  return next;
}

export function CardGrid({
  cards,
  editing,
  labelOf,
  render,
  onChange,
  onAdd,
}: {
  cards: Placed[];
  editing: boolean;
  labelOf: (id: string) => string;
  render: (id: string) => ReactNode;
  /** A finished change: a drop, a new width, a card taken off. */
  onChange: (cards: Placed[]) => void;
  onAdd: () => void;
}) {
  const grid = useRef<HTMLDivElement>(null);
  const slots = useRef(new Map<string, HTMLDivElement>());
  // The arrangement while a drag or a resize is under way.
  const [draft, setDraft] = useState<Placed[] | null>(null);
  const [dragging, setDragging] = useState<string>();
  const [said, say] = useState("");
  // Moving a card moves its element, which would play its entrance again:
  // after the first move the charts stay drawn (settled), and in this round
  // of arranging the edges stay put (still).
  const [settled, setSettled] = useState(false);
  const [still, setStill] = useState(false);
  useEffect(() => {
    if (!editing) setStill(false);
  }, [editing]);
  const drag = useRef<Drag | null>(null);
  const resize = useRef<Resize | null>(null);
  const before = useRef<Map<string, DOMRect> | null>(null);
  const scroll = useRef<{ speed: number; frame?: number }>({ speed: 0 });
  const hint = useId();
  const list = draft ?? cards;
  // The order as of the last change, ahead of the render that shows it:
  // pointer events can come faster than renders.
  const current = useRef(list);
  current.current = list;
  const setOrder = (next: Placed[] | null) => {
    current.current = next ?? cards;
    setDraft(next);
  };

  /** Where each card is on screen now, so the next layout can slide from there. */
  const snapshot = () => {
    const rects = new Map<string, DOMRect>();
    slots.current.forEach((el, id) => rects.set(id, el.getBoundingClientRect()));
    before.current = rects;
    setSettled(true);
    setStill(true);
  };

  /** Keep the dragged card under the pointer, at the spot it was taken by. */
  const follow = () => {
    const d = drag.current;
    const el = d && slots.current.get(d.id);
    const box = grid.current?.getBoundingClientRect();
    if (!d?.live || !el || !box) return;
    const x = d.x - box.left - d.grabX - el.offsetLeft;
    const y = d.y - box.top - d.grabY - el.offsetTop;
    el.style.transform = `translate(${x}px, ${y}px)`;
  };

  // After a change of order or width, the other cards slide from where
  // they were (mid-slide included) to where they now sit.
  useLayoutEffect(() => {
    const rects = before.current;
    before.current = null;
    if (rects && !reducedMotion()) {
      slots.current.forEach((el, id) => {
        if (drag.current?.live && id === drag.current.id) return;
        const was = rects.get(id);
        if (!was) return;
        el.getAnimations().forEach((a) => a.cancel());
        const now = el.getBoundingClientRect();
        const dx = was.left - now.left;
        const dy = was.top - now.top;
        if (Math.abs(dx) < 1 && Math.abs(dy) < 1) return;
        el.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: "none" }], {
          duration: 260,
          easing: EASE,
        });
      });
    }
    follow();
  });

  // A finger that picked a card up moves it, not the page.
  useEffect(() => {
    const el = grid.current;
    if (!el) return;
    const hold = (e: TouchEvent) => {
      if (drag.current?.live) e.preventDefault();
    };
    el.addEventListener("touchmove", hold, { passive: false });
    return () => el.removeEventListener("touchmove", hold);
  }, []);

  useEffect(() => () => stopScrolling(), []);

  /** The card whose place is under the pointer, by layout, not by any slide in progress. */
  const indexAt = (x: number, y: number, except: string): number | undefined => {
    const box = grid.current?.getBoundingClientRect();
    if (!box) return undefined;
    for (const [id, el] of slots.current) {
      if (id === except) continue;
      const left = box.left + el.offsetLeft;
      const top = box.top + el.offsetTop;
      if (x >= left && x < left + el.offsetWidth && y >= top && y < top + el.offsetHeight) {
        return current.current.findIndex((card) => card.id === id);
      }
    }
    return undefined;
  };

  const place = () => {
    const d = drag.current;
    if (!d?.live) return;
    const order = current.current;
    const to = indexAt(d.x, d.y, d.id);
    const over = to === undefined ? undefined : order[to]?.id;
    if (over === undefined || over === d.passed) {
      if (over === undefined) d.passed = undefined;
      return;
    }
    const from = order.findIndex((card) => card.id === d.id);
    if (from < 0 || to === undefined || to < 0 || to === from) return;
    d.passed = over;
    snapshot();
    setOrder(moved(order, from, to));
  };

  function stopScrolling() {
    if (scroll.current.frame) cancelAnimationFrame(scroll.current.frame);
    scroll.current = { speed: 0 };
  }

  const scrollNear = (y: number) => {
    const speed =
      y < EDGE_PX ? -(EDGE_PX - y) / 4 : y > window.innerHeight - EDGE_PX ? (y - window.innerHeight + EDGE_PX) / 4 : 0;
    scroll.current.speed = speed;
    if (!speed || scroll.current.frame) return;
    const step = () => {
      if (!scroll.current.speed || !drag.current?.live) {
        stopScrolling();
        return;
      }
      window.scrollBy(0, scroll.current.speed);
      follow();
      place();
      scroll.current.frame = requestAnimationFrame(step);
    };
    scroll.current.frame = requestAnimationFrame(step);
  };

  const pickUp = (el: HTMLElement) => {
    const d = drag.current;
    if (!d) return;
    d.live = true;
    try {
      el.setPointerCapture(d.pointerId);
    } catch {
      // The pointer already left; the drag ends with it.
    }
    setOrder(current.current);
    setDragging(d.id);
    follow();
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>, id: string) => {
    if (!editing || !e.isPrimary || e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button, [role='slider']")) return;
    const el = e.currentTarget;
    const box = el.getBoundingClientRect();
    drag.current = {
      id,
      pointerId: e.pointerId,
      touch: e.pointerType === "touch",
      startX: e.clientX,
      startY: e.clientY,
      grabX: e.clientX - box.left,
      grabY: e.clientY - box.top,
      x: e.clientX,
      y: e.clientY,
      live: false,
    };
    if (e.pointerType === "touch") drag.current.hold = window.setTimeout(() => pickUp(el), HOLD_MS);
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || e.pointerId !== d.pointerId) return;
    d.x = e.clientX;
    d.y = e.clientY;
    if (!d.live) {
      const far = Math.hypot(e.clientX - d.startX, e.clientY - d.startY);
      if (d.touch) {
        // Moved before the hold: a swipe, which scrolls.
        if (far > HOLD_SLOP_PX) {
          window.clearTimeout(d.hold);
          drag.current = null;
        }
        return;
      }
      if (far < LIFT_PX) return;
      pickUp(e.currentTarget);
      return;
    }
    follow();
    place();
    scrollNear(e.clientY);
  };

  const onPointerEnd = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || e.pointerId !== d.pointerId) return;
    window.clearTimeout(d.hold);
    drag.current = null;
    stopScrolling();
    if (!d.live) return;
    // The card settles into its place from where it was let go.
    const el = slots.current.get(d.id);
    if (el) {
      const from = el.style.transform;
      el.style.transform = "";
      if (from && !reducedMotion()) {
        el.animate([{ transform: from }, { transform: "none" }], { duration: 220, easing: EASE });
      }
    }
    setDragging(undefined);
    const next = current.current;
    setOrder(null);
    if (!same(next, cards)) {
      const at = next.findIndex((card) => card.id === d.id);
      say(s.moved(labelOf(d.id), at + 1, next.length));
      onChange(next);
    }
  };

  /** Move a focused card with the arrow keys. */
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>, id: string) => {
    if (!editing || e.target !== e.currentTarget) return;
    const by = { ArrowLeft: -1, ArrowUp: -1, ArrowRight: 1, ArrowDown: 1 }[e.key];
    if (!by) return;
    e.preventDefault();
    const from = cards.findIndex((card) => card.id === id);
    const to = from + by;
    if (from < 0 || to < 0 || to >= cards.length) return;
    snapshot();
    say(s.moved(labelOf(id), to + 1, cards.length));
    onChange(moved(cards, from, to));
  };

  const widen = (id: string, width: number, commit: boolean) => {
    const w = Math.max(NARROWEST, Math.min(COLUMNS, width));
    const base = current.current;
    if (base.find((card) => card.id === id)?.width === w && !commit) return;
    snapshot();
    const next = base.map((card) => (card.id === id ? { ...card, width: w } : card));
    if (commit) {
      setOrder(null);
      if (!same(next, cards)) onChange(next);
    } else {
      setOrder(next);
    }
  };

  /** The width the pointer's place gives a card, in whole columns. */
  const widthAt = (x: number, left: number): number => {
    const el = grid.current;
    if (!el) return NARROWEST;
    const gap = parseFloat(getComputedStyle(el).columnGap) || 0;
    const column = (el.getBoundingClientRect().width + gap) / COLUMNS;
    return Math.round((x - left + gap) / column);
  };

  const onSizeDown = (e: PointerEvent<HTMLSpanElement>, id: string) => {
    if (!e.isPrimary || e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const slot = slots.current.get(id);
    if (!slot) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    resize.current = { id, pointerId: e.pointerId, left: slot.getBoundingClientRect().left };
    setOrder(cards);
  };
  const onSizeMove = (e: PointerEvent<HTMLSpanElement>) => {
    const r = resize.current;
    if (!r || e.pointerId !== r.pointerId) return;
    widen(r.id, widthAt(e.clientX, r.left), false);
  };
  const onSizeEnd = (e: PointerEvent<HTMLSpanElement>) => {
    const r = resize.current;
    if (!r || e.pointerId !== r.pointerId) return;
    resize.current = null;
    const width = current.current.find((card) => card.id === r.id)?.width;
    if (width !== undefined) widen(r.id, width, true);
    else setOrder(null);
  };
  const onSizeKey = (e: KeyboardEvent<HTMLSpanElement>, card: Placed) => {
    const next = {
      ArrowLeft: card.width - 1,
      ArrowDown: card.width - 1,
      ArrowRight: card.width + 1,
      ArrowUp: card.width + 1,
      Home: NARROWEST,
      End: COLUMNS,
    }[e.key];
    if (next === undefined) return;
    e.preventDefault();
    e.stopPropagation();
    widen(card.id, next, true);
  };

  const remove = (id: string) => {
    snapshot();
    say(s.removed(labelOf(id)));
    onChange(cards.filter((card) => card.id !== id));
  };

  return (
    <div
      ref={grid}
      className={[
        "screen-dash screen-cards",
        editing && "is-editing",
        settled && "is-settled",
        editing && still && "is-still",
      ]
        .filter(Boolean)
        .join(" ")}
      role={editing ? "list" : undefined}
      aria-label={editing ? s.cardsLabel : undefined}
      aria-describedby={editing ? hint : undefined}
    >
      {list.map((card, i) => {
        const label = labelOf(card.id);
        return (
          <div
            key={card.id}
            ref={(el) => {
              if (el) slots.current.set(card.id, el);
              else slots.current.delete(card.id);
            }}
            className={dragging === card.id ? "screen-slot is-dragging" : "screen-slot"}
            style={{ "--span": card.width } as CSSProperties}
            role={editing ? "listitem" : undefined}
            tabIndex={editing ? 0 : undefined}
            aria-label={editing ? s.cardAt(label, i + 1, list.length) : undefined}
            onPointerDown={(e) => onPointerDown(e, card.id)}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerEnd}
            onPointerCancel={onPointerEnd}
            onKeyDown={(e) => onKeyDown(e, card.id)}
          >
            <div className="screen-slot__card" inert={editing}>
              {render(card.id)}
            </div>
            {editing && (
              <>
                <IconButton
                  icon="minus"
                  filled
                  className="screen-slot__remove"
                  label={s.removeCard(label)}
                  onClick={() => remove(card.id)}
                />
                <span
                  role="slider"
                  tabIndex={0}
                  className="screen-slot__size"
                  aria-label={s.widthOf(label)}
                  aria-orientation="horizontal"
                  aria-valuemin={NARROWEST}
                  aria-valuemax={COLUMNS}
                  aria-valuenow={card.width}
                  aria-valuetext={s.columns(card.width)}
                  onPointerDown={(e) => onSizeDown(e, card.id)}
                  onPointerMove={onSizeMove}
                  onPointerUp={onSizeEnd}
                  onPointerCancel={onSizeEnd}
                  onKeyDown={(e) => onSizeKey(e, card)}
                />
              </>
            )}
          </div>
        );
      })}
      {editing && (
        <button type="button" className="screen-slot screen-slot--add" onClick={onAdd}>
          <Icon name="plus" />
          {s.addCard}
        </button>
      )}
      {editing && (
        <p id={hint} className="oik-visually-hidden">
          {s.arrangeHint}
        </p>
      )}
      <span className="oik-visually-hidden" aria-live="polite">
        {said}
      </span>
    </div>
  );
}

/** Put a card on Home: what Oikome makes, and later what plugins make. */
export function AddCardDialog({
  choices,
  shown,
  onAdd,
  onClose,
}: {
  choices: CardChoice[];
  shown: string[];
  onAdd: (choice: CardChoice) => void;
  onClose: () => void;
}) {
  return (
    <Dialog
      title={s.addCard}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <Button variant="primary" onClick={onClose}>
          {strings.common.done}
        </Button>
      }
    >
      {[
        { title: s.fromOikome, cards: choices.filter((c) => !c.id.startsWith("plugin:")) },
        { title: s.fromPlugins, cards: choices.filter((c) => c.id.startsWith("plugin:")) },
      ].map(({ title, cards }) => (
        <section key={title} className="screen-add-card__group">
          <h3 className="oik-label screen-add-card__label">{title}</h3>
          <ul className="screen-add-card">
            {cards.map((choice) => (
              <li key={choice.id} className="screen-add-card__row">
                <Tile icon={choice.icon} tone="stone" />
                <span className="screen-add-card__name">{choice.label}</span>
                {shown.includes(choice.id) ? (
                  <Tag>{s.onHome}</Tag>
                ) : (
                  <Button onClick={() => onAdd(choice)}>{s.add}</Button>
                )}
              </li>
            ))}
            {cards.length === 0 && (
              <li className="screen-add-card__row">
                <span className="screen-add-card__name screen-add-card__none">{s.noPlugins}</span>
                <Link to="/plugins" className="oik-link">
                  {s.browsePlugins}
                </Link>
              </li>
            )}
          </ul>
        </section>
      ))}
    </Dialog>
  );
}
