// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Pieces several screens share: a Back link, a table of rows and the
// recovery key panel.

import { useState } from "react";
import type { ReactNode } from "react";
import { Button, Checkbox, CopyField, ErrorText, Icon, InsetWell } from "@oikome/ui";
import type { NodeInfo } from "@oikome/client";
import { DataType } from "apache-arrow";
import type { Field, Table } from "apache-arrow";
import { useAction, useClient, useLoad } from "./data";
import { calendarDate, cell, columnLabel, dateTime, decimal, wallTime } from "./format";
import { navigate, plainClick, previousPage } from "./router";
import { strings } from "./strings";

/** A screen's title, a small label over it and a line under it, actions on the right. */
export function ScreenHeader({
  title,
  label,
  lede,
  mono,
  children,
}: {
  title: ReactNode;
  label?: ReactNode;
  lede?: ReactNode;
  /** A collection's name: set in the data face. */
  mono?: boolean;
  children?: ReactNode;
}) {
  return (
    <header className="screen-header">
      <div className="screen-header__text">
        {label && <span className="oik-label">{label}</span>}
        <h1 className={mono ? "screen-title screen-title--mono" : "screen-title"}>{title}</h1>
        {lede && <p className="screen-lede">{lede}</p>}
      </div>
      {children && <div className="screen-header__actions">{children}</div>}
    </header>
  );
}

/** What a page is called in a Back link. */
function pageTitle(url: string): string | undefined {
  const path = new URL(url, window.location.origin).pathname;
  if (path.startsWith("/collections/")) {
    try {
      return decodeURIComponent(path.slice("/collections/".length));
    } catch {
      return undefined;
    }
  }
  const titles: Record<string, string> = {
    "/": strings.nav.home,
    "/collections": strings.nav.collections,
    "/sharing": strings.nav.sharing,
    "/activity": strings.nav.activity,
    "/settings": strings.nav.settings,
  };
  return titles[path];
}

/**
 * Back to the page this one was opened from, named after it. A page opened
 * directly (a bookmark, a reload of a fresh tab) goes up to `parent`.
 */
export function BackLink({
  parent,
  parentLabel,
  bare,
}: {
  parent: string;
  parentLabel: string;
  /** In a breadcrumb: the name alone, no arrow. */
  bare?: boolean;
}) {
  const from = previousPage();
  const title = from ? pageTitle(from) : undefined;
  return (
    <a
      href={title ? from : parent}
      className={bare ? "screen-crumb__link screen-back" : "oik-link screen-back"}
      onClick={(e) => {
        if (!plainClick(e)) return;
        e.preventDefault();
        if (title) window.history.back();
        else navigate(parent);
      }}
    >
      {!bare && <Icon name="back" />}
      {title ?? parentLabel}
    </a>
  );
}

/** A value of a column as text; dates and times the way people write them. */
export function show(field: Field, value: unknown): string | null {
  if (value === null || value === undefined) return null;
  if (DataType.isDate(field.type)) return calendarDate(Number(value));
  if (DataType.isTimestamp(field.type)) {
    return field.type.timezone ? dateTime(Number(value)) : wallTime(Number(value));
  }
  if (DataType.isDecimal(field.type)) return decimal(String(value), field.type.scale);
  return cell(value);
}

/** Integer, float and decimal columns: right-aligned so digits line up. */
export function numeric(field: Field): boolean {
  return DataType.isInt(field.type) || DataType.isFloat(field.type) || DataType.isDecimal(field.type);
}

/** Rows of an Arrow table; empty cells show a dash. */
export function RowsTable({ table, label }: { table: Table; label: string }) {
  const fields = table.schema.fields;
  const rows = Array.from({ length: table.numRows }, (_, r) =>
    fields.map((field, c) => show(field, table.getChildAt(c)?.get(r))),
  );
  return (
    <div className="oik-table-wrap">
      <table className="oik-table" aria-label={label}>
        <thead>
          <tr>
            {fields.map((f) => (
              <th key={f.name} scope="col" title={f.name} className={numeric(f) ? "oik-num" : undefined}>
                {columnLabel(f.name)}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr key={i}>
              {row.map((value, j) => (
                <td key={j} className={numeric(fields[j]!) ? "oik-num" : undefined}>
                  {value === null ? (
                    <span className="oik-table__null" aria-label="empty">
                      —
                    </span>
                  ) : (
                    value
                  )}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function download(name: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: "text/plain" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  document.body.append(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

/** Show the recovery key, and record that the owner stored it. */
export function RecoveryKeyPanel({
  doneLabel,
  onDone,
  onLater,
  laterLabel = strings.recovery.later,
}: {
  doneLabel: string;
  onDone: (info: NodeInfo) => void;
  onLater?: () => void;
  laterLabel?: string;
}) {
  const client = useClient();
  const key = useLoad(() => client.recoveryKey(), []);
  const [stored, setStored] = useState(false);
  const confirm = useAction();
  return (
    <div className="screen-recovery">
      <InsetWell>
        {key.data ? (
          <CopyField
            value={key.data.key}
            copyLabel={strings.common.copy}
            copiedLabel={strings.common.copied}
          />
        ) : key.error ? (
          <ErrorText>{key.error}</ErrorText>
        ) : (
          <p className="screen-muted">{strings.common.loading}</p>
        )}
        <p className="screen-muted screen-recovery__hint">{strings.recovery.hint}</p>
      </InsetWell>
      <Checkbox label={strings.recovery.stored} checked={stored} onChange={setStored} />
      {confirm.error && <ErrorText>{confirm.error}</ErrorText>}
      <div className="screen-actions">
        {/* One crimson action at a time: save the key first, then go on. */}
        {stored ? (
          <Button
            variant="primary"
            large
            disabled={!key.data || confirm.busy}
            onClick={async () => {
              const info = await confirm.run(() => client.confirmRecoveryKey());
              if (info) onDone(info);
            }}
          >
            {doneLabel}
          </Button>
        ) : (
          <Button
            variant="primary"
            large
            icon="download"
            disabled={!key.data}
            onClick={() =>
              download(
                strings.recovery.fileName,
                `${strings.recovery.fileIntro}\n\n${key.data?.key ?? ""}\n`,
              )
            }
          >
            {strings.recovery.download}
          </Button>
        )}
        {onLater && (
          <Button variant="text" onClick={onLater}>
            {laterLabel}
          </Button>
        )}
      </div>
    </div>
  );
}
