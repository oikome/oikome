// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The ⌘K palette: go anywhere and do the common things by typing. It finds
// pages, collections and contacts by name; searching inside what they hold
// comes with search (M3).

import { useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent, ReactNode } from "react";
import { Icon } from "@oikome/ui";
import type { IconName } from "@oikome/ui";
import { useClient, useLoad } from "./data";
import { navigate } from "./router";
import { strings } from "./strings";
import { applyTheme } from "./theme";

const s = strings.palette;

interface Entry {
  key: string;
  group: string;
  icon: IconName;
  title: string;
  meta?: string;
  /** Other words it is found by. */
  words?: string;
  run: () => void;
}

/** Wrap the first match of `query` in `title` in a mark. */
function marked(title: string, query: string): ReactNode {
  const i = query ? title.toLowerCase().indexOf(query.toLowerCase()) : -1;
  if (i < 0) return title;
  return (
    <>
      {title.slice(0, i)}
      <mark>{title.slice(i, i + query.length)}</mark>
      {title.slice(i + query.length)}
    </>
  );
}

export function Palette({ onClose }: { onClose: () => void }) {
  const client = useClient();
  const spaces = useLoad(() => client.listSpaces(), []);
  const contacts = useLoad(() => client.listContacts(), []);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const dialog = useRef<HTMLDialogElement>(null);
  const list = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = dialog.current;
    if (el && !el.open) el.showModal();
    return () => el?.close();
  }, []);

  const go = (to: string) => () => navigate(to);
  const entries = useMemo<Entry[]>(() => {
    const out: Entry[] = [];
    for (const space of spaces.data ?? []) {
      out.push({
        key: `c:${space.name}`,
        group: s.collections,
        icon: space.kind === "remote" ? "person-plus" : "collection",
        title: space.name,
        meta: space.kind === "remote" ? strings.collections.fromElsewhere : space.example ? strings.collections.example : undefined,
        run: go(`/collections/${encodeURIComponent(space.name)}`),
      });
    }
    for (const contact of contacts.data ?? []) {
      out.push({
        key: `p:${contact.did}`,
        group: s.people,
        icon: "person-plus",
        title: contact.name,
        meta: s.contact,
        run: go("/sharing"),
      });
    }
    out.push(
      { key: "a:new", group: s.actions, icon: "plus", title: strings.collections.newCollection, words: "create add", run: go("/collections?new=1") },
      { key: "a:share", group: s.actions, icon: "person-plus", title: strings.sharing.newShare, words: "share invite", run: go("/sharing?share=1") },
      { key: "a:contact", group: s.actions, icon: "person-plus", title: strings.sharing.addContact, words: "person friend", run: go("/sharing?add=1") },
      { key: "a:key", group: s.actions, icon: "key", title: strings.backup.backUpNow, words: "recovery key backup", run: go("/settings?backup=1") },
      { key: "a:ink", group: s.actions, icon: "sliders", title: s.useInk, words: "style theme appearance", run: () => applyTheme("ink") },
      { key: "a:bold", group: s.actions, icon: "sliders", title: s.useBold, words: "style theme appearance", run: () => applyTheme("bold") },
      { key: "g:home", group: s.pages, icon: "home", title: strings.nav.home, run: go("/") },
      { key: "g:collections", group: s.pages, icon: "collection", title: strings.nav.collections, run: go("/collections") },
      { key: "g:sharing", group: s.pages, icon: "person-plus", title: strings.nav.sharing, run: go("/sharing") },
      { key: "g:activity", group: s.pages, icon: "activity", title: strings.nav.activity, run: go("/activity") },
      { key: "g:settings", group: s.pages, icon: "sliders", title: strings.nav.settings, words: "storage recovery appearance", run: go("/settings") },
    );
    return out;
  }, [spaces.data, contacts.data]);

  const q = query.trim().toLowerCase();
  const found = q
    ? entries.filter((e) => `${e.title} ${e.meta ?? ""} ${e.words ?? ""}`.toLowerCase().includes(q))
    : entries.filter((e) => e.group !== s.people);
  const groups = [s.collections, s.people, s.actions, s.pages]
    .map((group) => ({ group, items: found.filter((e) => e.group === group) }))
    .filter((g) => g.items.length > 0);
  const flat = groups.flatMap((g) => g.items);
  const current = Math.min(active, Math.max(0, flat.length - 1));

  const run = (entry: Entry | undefined) => {
    if (!entry) return;
    onClose();
    entry.run();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = (current + (e.key === "ArrowDown" ? 1 : -1) + flat.length) % Math.max(1, flat.length);
      setActive(next);
      list.current?.querySelector(`[data-index="${next}"]`)?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter") {
      e.preventDefault();
      run(flat[current]);
    }
  };

  let index = -1;
  return (
    <dialog
      ref={dialog}
      className="screen-palette"
      aria-label={s.title}
      onClose={onClose}
      onClick={(e) => {
        if (e.target === dialog.current) onClose();
      }}
    >
      <div className="screen-palette__input">
        <Icon name="search" />
        <input
          autoFocus
          value={query}
          placeholder={s.placeholder}
          aria-label={s.title}
          aria-controls="palette-list"
          aria-activedescendant={flat[current] ? `palette-${flat[current]!.key}` : undefined}
          role="combobox"
          aria-expanded="true"
          onChange={(e) => {
            setQuery(e.target.value);
            setActive(0);
          }}
          onKeyDown={onKey}
        />
        <kbd>esc</kbd>
      </div>
      <div ref={list} id="palette-list" role="listbox" className="screen-palette__list">
        {flat.length === 0 && <p className="screen-palette__empty">{s.nothing(query.trim())}</p>}
        {groups.map(({ group, items }) => (
          <div key={group} role="group" aria-label={group} className="screen-palette__group">
            <span className="oik-label">{group}</span>
            {items.map((entry) => {
              index++;
              const i = index;
              return (
                <div
                  key={entry.key}
                  id={`palette-${entry.key}`}
                  role="option"
                  aria-selected={i === current}
                  data-index={i}
                  className="screen-palette__hit"
                  onMouseMove={() => setActive(i)}
                  onClick={() => run(entry)}
                >
                  <span className="oik-tile">
                    <Icon name={entry.icon} />
                  </span>
                  <span className="screen-palette__text">
                    <span className="screen-palette__title" translate={entry.group === s.collections ? "no" : undefined}>
                      {marked(entry.title, query.trim())}
                    </span>
                    {entry.meta && <span className="screen-palette__meta">{entry.meta}</span>}
                  </span>
                  {i === current && <span className="screen-palette__enter">{s.open}</span>}
                </div>
              );
            })}
          </div>
        ))}
      </div>
    </dialog>
  );
}
