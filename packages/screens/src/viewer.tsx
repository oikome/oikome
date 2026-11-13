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
import { tableText, writeClipboard } from "./clipboard";
import { itemDate } from "./covers";
import { rawMessage, useAction, useClient, useLoad } from "./data";
import { bytes, dayAndTime } from "./format";
import type { ItemNav } from "./item";
import { strings } from "./strings";

const MarkdownView = lazy(() => import("./MarkdownView"));
const SourceView = lazy(() => import("./SourceView"));
const DataGrid = lazy(() => import("./DataGrid"));
const ImageView = lazy(() => import("./ImageView"));
const SafeSvgImage = lazy(() => import("./SafeSvgImage"));

const s = strings.collections;
const PAGE_ROWS = 100;

/** What a file is, in a word: a data file or original medium by its format. */
export function fileKindLabel(name: string, kind: string, mediaType?: string | null): string {
  const extension = name.split(".").pop()?.toLowerCase() ?? "";
  if (kind === "table") {
    return extension === "parquet" ? "Parquet" : extension.startsWith("json") || extension === "ndjson" ? "JSON" : "CSV";
  }
  if (mediaType?.startsWith("audio/")) return s.fileKinds.audio!;
  if (mediaType?.startsWith("video/")) return s.fileKinds.video!;
  if (mediaType === "image/svg+xml") return s.fileKinds.vector!;
  return s.fileKinds[kind] ?? s.fileKinds.other!;
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
  facts: { cameFrom: ReactNode; cameFromPath?: (path: string) => ReactNode; keptOn: ReactNode; who: string };
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
  const [copied, setCopied] = useState(false);
  const [mediaError, setMediaError] = useState(false);
  const reveal = useAction();
  const copy = useAction();
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
  useEffect(() => {
    setOffset(0);
    setMediaError(false);
  }, [path]);
  useEffect(() => setCopied(false), [path, view, offset]);
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
  const selectAll = useRef(false);
  const currentText = useRef("");
  moving.current = nav;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "c" && selectAll.current) {
        e.preventDefault();
        e.stopPropagation();
        selectAll.current = false;
        void copy.run(async () => {
          await writeClipboard(currentText.current);
          return true;
        }).then((done) => {
          if (done) setCopied(true);
        });
        return;
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "a") {
        const content = ref.current?.querySelector<HTMLElement>(".cm-content, .screen-fileview__note, .screen-data-grid");
        if (content) {
          e.preventDefault();
          e.stopPropagation();
          selectAll.current = true;
          const range = document.createRange();
          range.selectNodeContents(content);
          const selection = window.getSelection();
          selection?.removeAllRanges();
          selection?.addRange(range);
        }
        return;
      }
      if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey || e.defaultPrevented || document.querySelector(".pswp--open")) return;
      if (e.target instanceof HTMLElement && e.target.closest("audio, video, input, textarea, [contenteditable='true']")) return;
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
  const canCopy = file?.text != null || (shownView === "table" && rows.data !== undefined);
  currentText.current = shownView === "table" && rows.data ? tableText(rows.data.table) : (file?.text ?? "");
  const copyCurrent = async () => {
    if (!file) return;
    setCopied(false);
    const text = currentText.current;
    if (!canCopy) return;
    const done = await copy.run(async () => {
      await writeClipboard(text);
      return true;
    });
    if (done) setCopied(true);
  };
  const cameFrom = file ? (facts.cameFromPath?.(file.path) ?? facts.cameFrom) : facts.cameFrom;
  const factsList: [string, ReactNode][] = file
    ? [
        [s.in_, space],
        [s.colKind, fileKindLabel(file.name, file.kind, file.content.mediaType)],
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
              {cameFrom}
            </button>
          ) : (
            cameFrom
          ),
        ],
      ]
    : [];

  return (
    <dialog
      ref={ref}
      className={`screen-item screen-fileview screen-fileview--${file?.kind ?? "other"}${info ? " screen-fileview--info" : ""}`}
      aria-labelledby={titleId}
      onPointerDown={() => {
        selectAll.current = false;
      }}
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
        {file && (viewOptions.length > 1 || canCopy) && (
          <div className="screen-fileview__viewbar">
            {viewOptions.length > 1 && (
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
            {canCopy && (
              <Button icon="copy" variant="quiet" disabled={copy.busy} onClick={() => void copyCurrent()}>
                {copied ? strings.common.copied : strings.common.copy}
              </Button>
            )}
            {copy.error && <ErrorText>{copy.error}</ErrorText>}
          </div>
        )}
        <div className="screen-fileview__body">
          {detail.error && <ErrorText>{detail.error}</ErrorText>}
          {reveal.error && <ErrorText>{reveal.error}</ErrorText>}
          {!file && !detail.error && <div className="screen-item__skeleton oik-skel" aria-hidden="true" />}
          {file?.content.views.includes("image") && (
            <div className="screen-fileview__image">
              <Suspense fallback={<div className="screen-item__skeleton oik-skel" aria-hidden="true" />}>
                {file.content.format === "svg" ? (
                  <SafeSvgImage src={client.fileUrl("original", { space, path: file.path })} alt={file.title ?? file.name} />
                ) : (
                  <ImageView
                    src={client.fileUrl("original", { space, path: file.path })}
                    alt={file.title ?? file.name}
                    gallery={nav?.gallery}
                  />
                )}
              </Suspense>
            </div>
          )}
          {file?.content.views.includes("audio") && (
            <div className="screen-fileview__media">
              <audio
                key={file.path}
                controls
                preload="metadata"
                aria-label={file.name}
                src={client.fileUrl("original", { space, path: file.path })}
                onError={() => setMediaError(true)}
              />
              {mediaError && <ErrorText>{s.mediaUnavailable}</ErrorText>}
            </div>
          )}
          {file?.content.views.includes("video") && (
            <div className="screen-fileview__media screen-fileview__media--video">
              <video
                key={file.path}
                controls
                playsInline
                preload="metadata"
                aria-label={file.name}
                src={client.fileUrl("original", { space, path: file.path })}
                onError={() => setMediaError(true)}
              />
              {mediaError && <ErrorText>{s.mediaUnavailable}</ErrorText>}
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
                    <SourceView text={file.text} name={file.name} />
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
              <p>{fileKindLabel(file.name, file.kind, file.content.mediaType)}</p>
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
