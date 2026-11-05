// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The ⌘K palette: search what the collections hold, and go anywhere or do
// the common things by typing. Notes, photos and rows come from the node's
// search; pages, collections, people and actions are found by name. Opened
// from a collection's page, it searches only that collection.

import { useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent, ReactNode } from "react";
import { Icon } from "@oikome/ui";
import type { IconName } from "@oikome/ui";
import type { SearchHit, SearchMark, SearchResults, SpaceSummary } from "@oikome/client";
import { imageSrc, loadPreview, shapeOf } from "./content";
import type { View } from "./content";
import { errorMessage, useClient, useLoad } from "./data";
import type { NodeClient } from "@oikome/client";
import { columnLabel } from "./format";
import { itemPath } from "./item";
import { navigate } from "./router";
import { strings } from "./strings";
import { applyTheme } from "./theme";

const s = strings.palette;
/** How long typing pauses before the node is asked. */
const WAIT_MS = 180;
/** Hits asked for, and shown in each group. */
const HITS = 30;
const PER_GROUP = 4;
const MAX_QUERY = 200;

interface Entry {
  key: string;
  group: string;
  icon: IconName;
  title: string;
  meta?: string;
  /** Other words it is found by. */
  words?: string;
  /** Where the title matched, from the node's search. */
  marks?: SearchMark[];
  /** A photo hit, whose picture is read to show it. */
  photo?: SearchHit;
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

/** `text` with the node's marks, whose offsets count as JavaScript does. */
function withMarks(text: string, marks: SearchMark[]): ReactNode {
  const out: ReactNode[] = [];
  let at = 0;
  for (const { start, end } of marks) {
    if (start < at || end > text.length || end <= start) continue;
    out.push(text.slice(at, start), <mark key={start}>{text.slice(start, end)}</mark>);
    at = end;
  }
  out.push(text.slice(at));
  return out;
}

/** What each collection a search found in reads as, from its cover's read. */
type Views = Map<string, { table?: string; view: View }>;

interface Found {
  query: string;
  results?: SearchResults;
  views: Views;
  error?: string;
}

async function viewsOf(client: NodeClient, spaces: SpaceSummary[], hits: SearchHit[]): Promise<Views> {
  const names = [...new Set(hits.map((h) => h.collection))];
  const views: Views = new Map();
  await Promise.all(
    names.map(async (name) => {
      const space = spaces.find((x) => x.name === name);
      if (!space) return;
      try {
        const preview = await loadPreview(client, space);
        views.set(name, { table: preview.table?.name, view: preview.shape.view });
      } catch {
        // Read as rows when its cover could not be read.
      }
    }),
  );
  return views;
}

/** Pictures of photo hits, read once each while the app is open. */
const pictures = new Map<string, Promise<string | undefined>>();

function picture(client: NodeClient, hit: SearchHit): Promise<string | undefined> {
  const id = JSON.stringify([hit.collection, hit.table, hit.key]);
  let found = pictures.get(id);
  if (!found) {
    found = client
      .readRow({ space: hit.collection, table: hit.table, key: hit.key ?? [] })
      .then(({ table }) => {
        const image = shapeOf(table).image;
        const bytes: unknown = image === undefined ? undefined : table.getChildAt(image)?.get(0);
        return bytes instanceof Uint8Array ? imageSrc(bytes) : undefined;
      })
      .catch(() => {
        pictures.delete(id);
        return undefined;
      });
    pictures.set(id, found);
  }
  return found;
}

function Thumb({ hit }: { hit: SearchHit }) {
  const client = useClient();
  const [src, setSrc] = useState<string>();
  useEffect(() => {
    let live = true;
    if (hit.key) void picture(client, hit).then((url) => live && setSrc(url));
    return () => {
      live = false;
    };
  }, [client, hit]);
  return src ? (
    <img className="screen-palette__thumb" src={src} alt="" decoding="async" />
  ) : (
    <span className="oik-tile">
      <Icon name="photo" />
    </span>
  );
}

export function Palette({ onClose, scope }: { onClose: () => void; scope?: string }) {
  const client = useClient();
  const spaces = useLoad(() => client.listSpaces(), []);
  const contacts = useLoad(() => client.listContacts(), []);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [found, setFound] = useState<Found>();
  const dialog = useRef<HTMLDialogElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const asked = useRef(0);

  useEffect(() => {
    const el = dialog.current;
    if (el && !el.open) el.showModal();
    return () => el?.close();
  }, []);

  // Ask the node once typing pauses; an answer to an older query is dropped.
  const q = query.trim();
  useEffect(() => {
    const n = ++asked.current;
    if (!q || !spaces.data) {
      setFound(undefined);
      return;
    }
    const all = spaces.data;
    const timer = window.setTimeout(() => {
      void (async () => {
        let next: Found;
        try {
          const results = await client.search({ query: q, collection: scope, limit: HITS });
          next = { query: q, results, views: await viewsOf(client, all, results.hits) };
        } catch (e) {
          next = { query: q, views: new Map(), error: errorMessage(e) };
        }
        if (n === asked.current) setFound(next);
      })();
    }, WAIT_MS);
    return () => window.clearTimeout(timer);
  }, [client, q, scope, spaces.data]);

  const go = (to: string) => () => navigate(to);
  const entries = useMemo<Entry[]>(() => {
    const out: Entry[] = [];
    if (scope) return out;
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
      { key: "a:source", group: s.actions, icon: "download", title: strings.sources.addTitle, words: "import folder files csv photos notes obsidian sync", run: go("/sources?add=1") },
      { key: "a:share", group: s.actions, icon: "person-plus", title: strings.sharing.newShare, words: "share invite", run: go("/sharing?share=1") },
      { key: "a:contact", group: s.actions, icon: "person-plus", title: strings.sharing.addContact, words: "person friend", run: go("/sharing?add=1") },
      { key: "a:lend", group: s.actions, icon: "cpu", title: strings.sharing.lendTitle, words: "compute borrow cpu share computer", run: go("/sharing?lend=1") },
      { key: "a:invite", group: s.actions, icon: "person-plus", title: strings.sharing.addInvite, words: "invite code accept join shared", run: go("/sharing?invite=1") },
      { key: "a:key", group: s.actions, icon: "key", title: strings.backup.backUpNow, words: "recovery key backup", run: go("/settings?backup=1") },
      { key: "a:ink", group: s.actions, icon: "sliders", title: s.useInk, words: "style theme appearance", run: () => applyTheme("ink") },
      { key: "a:bold", group: s.actions, icon: "sliders", title: s.useBold, words: "style theme appearance", run: () => applyTheme("bold") },
      { key: "g:home", group: s.pages, icon: "home", title: strings.nav.home, run: go("/") },
      { key: "g:collections", group: s.pages, icon: "collection", title: strings.nav.collections, run: go("/collections") },
      { key: "g:sources", group: s.pages, icon: "download", title: strings.nav.sources, words: "files folders sync", run: go("/sources") },
      { key: "g:sharing", group: s.pages, icon: "person-plus", title: strings.nav.sharing, run: go("/sharing") },
      { key: "g:plugins", group: s.pages, icon: "plugin", title: strings.nav.plugins, words: "cards dashboard home add", run: go("/plugins") },
      { key: "g:activity", group: s.pages, icon: "activity", title: strings.nav.activity, run: go("/activity") },
      { key: "g:settings", group: s.pages, icon: "sliders", title: strings.nav.settings, words: "storage recovery appearance", run: go("/settings") },
    );
    return out;
  }, [scope, spaces.data, contacts.data]);

  // What the collections hold, grouped by what each reads as.
  const hits = useMemo<Entry[]>(() => {
    if (!found?.results) return [];
    return found.results.hits.map((hit, i) => {
      const known = found.views.get(hit.collection);
      const view: View = known && known.table === hit.table ? known.view : "table";
      const group = view === "photos" ? s.photos : view === "notes" ? s.notes : s.rows;
      const where = known && known.table !== hit.table ? s.inTable(hit.collection, hit.table) : hit.collection;
      const column = view === "table" || view === "series" ? columnLabel(hit.column) : undefined;
      const key = hit.key;
      return {
        key: `h:${i}:${hit.collection}:${hit.table}:${JSON.stringify(key)}`,
        group,
        icon: view === "photos" ? "photo" : view === "notes" ? "note" : "table",
        title: hit.excerpt,
        meta: column ? `${where} · ${column}` : where,
        marks: hit.marks,
        photo: view === "photos" ? hit : undefined,
        run: go(
          key
            ? itemPath({ collection: hit.collection, table: hit.table, key })
            : `/collections/${encodeURIComponent(hit.collection)}`,
        ),
      };
    });
  }, [found]);

  const lower = q.toLowerCase();
  const named = q
    ? entries.filter((e) => `${e.title} ${e.meta ?? ""} ${e.words ?? ""}`.toLowerCase().includes(lower))
    : entries.filter((e) => e.group !== s.people);
  const groups = [
    ...[s.notes, s.photos, s.rows].map((group) => ({
      group,
      items: q ? hits.filter((e) => e.group === group).slice(0, PER_GROUP) : [],
    })),
    ...[s.collections, s.people, s.actions, s.pages].map((group) => ({
      group,
      items: named.filter((e) => e.group === group),
    })),
  ].filter((g) => g.items.length > 0);
  const flat = groups.flatMap((g) => g.items);
  const current = Math.min(active, Math.max(0, flat.length - 1));
  const answered = found !== undefined && found.query === q;
  const skipped = found?.results?.skipped ?? [];
  const shared = skipped.filter((x) => x.reason === "shared_with_you").map((x) => x.collection);
  const failed = skipped.filter((x) => x.reason === "failed").map((x) => x.collection);

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
        {scope && (
          <span className="screen-palette__scope" translate="no">
            {s.in_(scope)}
          </span>
        )}
        <input
          autoFocus
          value={query}
          maxLength={MAX_QUERY}
          placeholder={scope ? s.placeholderIn(scope) : s.placeholder}
          aria-label={s.title}
          aria-controls="palette-list"
          aria-activedescendant={flat[current] ? `palette-${flat[current]!.key}` : undefined}
          role="combobox"
          aria-expanded="true"
          aria-autocomplete="list"
          autoComplete="off"
          spellCheck={false}
          onChange={(e) => {
            setQuery(e.target.value);
            setActive(0);
          }}
          onKeyDown={onKey}
        />
        <kbd>esc</kbd>
      </div>
      <div ref={list} id="palette-list" role="listbox" aria-label={s.title} className="screen-palette__list">
        {q && flat.length === 0 && (
          <p className="screen-palette__empty">
            {!answered ? s.searching : scope ? s.nothingIn(q, scope) : s.nothing(q)}
          </p>
        )}
        {groups.map(({ group, items }) => (
          <div key={group} role="group" aria-label={group} className="screen-palette__group">
            <span className="oik-label">{group}</span>
            {items.map((entry) => {
              index++;
              const i = index;
              const content = entry.marks !== undefined;
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
                  {entry.photo ? (
                    <Thumb hit={entry.photo} />
                  ) : (
                    <span className="oik-tile">
                      <Icon name={entry.icon} />
                    </span>
                  )}
                  <span className="screen-palette__text">
                    <span
                      className={content ? "screen-palette__title screen-palette__title--excerpt" : "screen-palette__title"}
                      translate={entry.group === s.collections ? "no" : undefined}
                    >
                      {content ? withMarks(entry.title, entry.marks!) : marked(entry.title, q)}
                    </span>
                    {entry.meta && (
                      <span className="screen-palette__meta" translate={content ? "no" : undefined}>
                        {entry.meta}
                      </span>
                    )}
                  </span>
                  {i === current && <span className="screen-palette__enter">{s.open}</span>}
                </div>
              );
            })}
          </div>
        ))}
        {q && answered && (found.error || failed.length > 0 || shared.length > 0) && (
          <div className="screen-palette__notes">
            {found.error && <p>{found.error}</p>}
            {failed.length > 0 && <p>{s.failed(failed)}</p>}
            {shared.length > 0 && <p>{s.notSearched(shared)}</p>}
          </div>
        )}
      </div>
      <p className="oik-visually-hidden" role="status" aria-live="polite">
        {q && answered ? s.found(hits.length) : ""}
      </p>
    </dialog>
  );
}
