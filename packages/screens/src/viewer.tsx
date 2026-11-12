// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// A file of a folder viewed in place, as the file itself: a photo large, a
// note in full, a data file as a table read where it is, and anything else
// by its name and size. Its own address opens it and ← → move through the
// folder; the quiet line keeps only when, where and who, with the rest Info.

import { lazy, Suspense, useEffect, useId, useRef, useState } from "react";
import type { ReactNode } from "react";
import { Button, ErrorText, Icon, IconButton, Segmented, Tag } from "@oikome/ui";
import type { FileItem } from "@oikome/client";
import { fileItems, tagsOf, withoutTagLines } from "./content";
import { itemDate } from "./covers";
import { rawMessage, useAction, useClient, useLoad } from "./data";
import { bytes, dayAndTime } from "./format";
import type { ItemNav } from "./item";
import { strings } from "./strings";

const MarkdownView = lazy(() => import("./MarkdownView"));
const SourceView = lazy(() => import("./SourceView"));
const DataGrid = lazy(() => import("./DataGrid"));

const s = strings.collections;
const PAGE_ROWS = 100;

/** What a file is, in a word: a data file by its format. */
export function fileKindLabel(name: string, kind: string): string {
  if (kind !== "table") return s.fileKinds[kind] ?? s.fileKinds.other!;
  const extension = name.split(".").pop()?.toLowerCase() ?? "";
  return extension === "parquet" ? "Parquet" : extension.startsWith("json") || extension === "ndjson" ? "JSON" : "CSV";
}

export function FileView({
  space,
  path,
  file: listed,
  facts,
  placeLabel,
  local,
  nav,
  onClose,
}: {
  space: string;
  path: string;
  /** The file as the folder's page already knows it; a direct address reads it. */
  file?: FileItem;
  facts: { cameFrom: ReactNode; keptOn: ReactNode; who: string };
  placeLabel: string;
  /** Whether this computer can reveal the file in its file manager. */
  local: boolean;
  nav?: ItemNav;
  onClose: () => void;
}) {
  const client = useClient();
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const infoId = useId();
  const [info, setInfo] = useState(false);
  const [offset, setOffset] = useState(0);
  const [view, setView] = useState("read");
  const [wrap, setWrap] = useState(false);
  const reveal = useAction();
  const detail = useLoad(
    () => client.readFile({ space, path }),
    [space, path],
    (e) => (e.code === "not_found" ? s.itemGone(space) : rawMessage(e)),
  );
  const file = detail.data ?? listed;
  const host = useLoad(() => client.nodeInfo(), []);
  const rows = useLoad(
    async () =>
      file?.content.views.includes("table")
        ? { ...(await client.readFileRows({ space, path, limit: PAGE_ROWS, offset })), at: offset }
        : undefined,
    [space, path, file?.content.views, offset],
    rawMessage,
  );

  useEffect(() => {
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
    return () => dialog?.close();
  }, []);
  useEffect(() => setOffset(0), [path]);
  useEffect(() => {
    let preferred = "read";
    try {
      preferred = localStorage.getItem(`oikome.file.view.${file?.content.format ?? ""}`) ?? preferred;
    } catch {
      // A private window may not keep the choice.
    }
    setView(preferred === "source" ? "source" : "read");
  }, [path, file?.content.format]);

  const moving = useRef(nav);
  moving.current = nav;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey || e.defaultPrevented) return;
      const go = e.key === "ArrowLeft" ? moving.current?.previous : e.key === "ArrowRight" ? moving.current?.next : undefined;
      if (!go) return;
      e.preventDefault();
      go();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);
  useEffect(() => {
    const dialog = ref.current;
    if (dialog?.open && !dialog.contains(document.activeElement)) {
      dialog.querySelector<HTMLButtonElement>(".screen-fileview__head button")?.focus();
    }
  }, [file]);

  const dated = file ? fileItems(client, space, [file])[0] : undefined;
  const when = dated ? itemDate(dated) : undefined;
  const changed = file?.modifiedMs ? dayAndTime(Number(file.modifiedMs)) : (file?.modified ?? "");
  const quiet = [when ?? changed, file?.place ?? placeLabel, facts.who].filter(Boolean).join(" · ");
  const tags = file?.kind === "note" ? tagsOf(file.text ?? undefined) : [];
  const availableViews = file?.content.views.filter((view) => view === "read" || view === "source" || view === "table") ?? [];
  const viewOptions = availableViews.map((value) => ({
    value,
    label: value === "read" ? s.readView : value === "source" ? s.sourceView : s.kinds.table,
  }));
  const shownView = availableViews.find((available) => available === view) ?? availableViews[0];
  const count = rows.data?.table.numRows ?? 0;
  const factsList: [string, ReactNode][] = file
    ? [
        [s.in_, space],
        [s.colKind, fileKindLabel(file.name, file.kind)],
        [s.colSize, bytes(Number(file.size))],
        [s.colModified, changed],
        [s.keptOn, facts.keptOn],
        [s.whoCanSee, facts.who],
        [
          s.cameFrom,
          local ? (
            <button
              type="button"
              className="oik-link screen-fileview__source"
              title={s.showInFinder(host.data?.platform ?? "")}
              disabled={reveal.busy}
              onClick={() => void reveal.run(() => client.showFolder({ space, path: file.path }))}
            >
              {facts.cameFrom}
            </button>
          ) : (
            facts.cameFrom
          ),
        ],
      ]
    : [];

  return (
    <dialog
      ref={ref}
      className={`screen-item screen-fileview screen-fileview--${file?.kind ?? "other"}`}
      aria-labelledby={titleId}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      onClick={(e) => {
        if (e.target === ref.current) onClose();
      }}
    >
      <header className="screen-fileview__head">
        <div className="screen-fileview__heading">
          <h2 id={titleId} className="screen-item__title" translate="no">
            {file?.name ?? path.split("/").pop()}
          </h2>
          <p className="screen-fileview__quiet">{quiet}</p>
        </div>
        <div className="screen-fileview__tools">
          {file && viewOptions.length > 1 && (
            <Segmented
              label={s.fileView}
              value={shownView ?? "read"}
              onChange={(next) => {
                setView(next);
                try {
                  localStorage.setItem(`oikome.file.view.${file.content.format}`, next);
                } catch {
                  // The choice still applies to this open viewer.
                }
              }}
              options={viewOptions}
            />
          )}
          {file && shownView === "source" && (
            <Button variant="quiet" aria-pressed={wrap} onClick={() => setWrap((on) => !on)}>
              {s.wrapLines}
            </Button>
          )}
          {local && file && (
            <Button variant="quiet" disabled={reveal.busy} onClick={() => void reveal.run(() => client.openWith({ space, path: file.path }))}>
              {s.openWith}
            </Button>
          )}
          <IconButton
            icon="info"
            label={s.info}
            aria-expanded={info}
            aria-controls={infoId}
            onClick={() => setInfo((shown) => !shown)}
          />
          <IconButton icon="close" label={strings.common.close} filled onClick={onClose} />
        </div>
      </header>

      {info && file && (
        <dl id={infoId} className="screen-item__facts screen-fileview__facts">
          {factsList.map(([label, value]) => (
            <div key={label}>
              <dt className="oik-label">{label}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>
      )}

      <div className="screen-fileview__content">
        {detail.error && <ErrorText>{detail.error}</ErrorText>}
        {reveal.error && <ErrorText>{reveal.error}</ErrorText>}
        {!file && !detail.error && <div className="screen-item__skeleton oik-skel" aria-hidden="true" />}
        {file?.kind === "image" && (
          <div className="screen-fileview__image">
            <img src={client.fileUrl("original", { space, path: file.path })} alt={file.title ?? file.name} />
          </div>
        )}
        {file && (shownView === "read" || shownView === "source") && (
          <div className={`screen-fileview__note ${shownView === "source" ? "screen-fileview__note--source" : ""}`}>
            {file.text == null ? (
              <ErrorText>{s.sourceUnavailable}</ErrorText>
            ) : (
              <Suspense fallback={<div className="screen-item__skeleton oik-skel" aria-hidden="true" />}>
                {shownView === "read" ? (
                  <>
                    <MarkdownView text={withoutTagLines(file.text) ?? ""} />
                    {tags.length > 0 && (
                      <div className="screen-tags">
                        {tags.map((tag) => (
                          <Tag key={tag}>#{tag}</Tag>
                        ))}
                      </div>
                    )}
                  </>
                ) : (
                  <SourceView text={file.text} name={file.name} wrap={wrap} />
                )}
              </Suspense>
            )}
          </div>
        )}
        {file && shownView === "table" && (
          <div className="screen-fileview__table">
            {rows.error && <ErrorText>{rows.error}</ErrorText>}
            {!rows.data && !rows.error && <div className="screen-item__skeleton oik-skel" aria-hidden="true" />}
            {rows.data && (
              <Suspense fallback={<div className="screen-item__skeleton oik-skel" aria-hidden="true" />}>
                <DataGrid table={rows.data.table} label={file.name} />
              </Suspense>
            )}
            {rows.data && count === 0 && <p className="screen-muted">{s.noRows}</p>}
            {rows.data?.truncated && <p className="screen-footnote">{s.fileCut}</p>}
            {(offset > 0 || count === PAGE_ROWS) && (
              <div className="screen-pager">
                <span className="screen-muted">{s.rows(offset + 1, offset + count)}</span>
                <span>
                  <Button variant="quiet" disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - PAGE_ROWS))}>
                    {s.previous}
                  </Button>
                  <Button variant="quiet" disabled={count < PAGE_ROWS} onClick={() => setOffset(offset + PAGE_ROWS)}>
                    {s.next}
                  </Button>
                </span>
              </div>
            )}
          </div>
        )}
        {file && file.content.views.length === 0 && (
          <div className="screen-fileview__other">
            <Icon name="collection" />
            <p>{fileKindLabel(file.name, file.kind)}</p>
            <p className="screen-muted">{bytes(Number(file.size))}</p>
            {file.content.unavailableReason && <p className="screen-muted">{file.content.unavailableReason}</p>}
            {local && (
              <Button icon="folder" disabled={reveal.busy} onClick={() => void reveal.run(() => client.showFolder({ space, path: file.path }))}>
                {s.showInFinder(host.data?.platform ?? "")}
              </Button>
            )}
          </div>
        )}
      </div>

      {nav && (
        <div className="screen-item__nav">
          <IconButton icon="back" label={s.previousItem} disabled={!nav.previous} onClick={() => nav.previous?.()} />
          <span className="screen-item__position">{s.position(nav.position, nav.total)}</span>
          <IconButton icon="chevron" label={s.nextItem} disabled={!nav.next} onClick={() => nav.next?.()} />
        </div>
      )}
    </dialog>
  );
}
