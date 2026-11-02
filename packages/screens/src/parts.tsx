// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Pieces several screens share: a table of rows and the recovery key panel.

import { useState } from "react";
import { Button, Checkbox, CopyField, ErrorText, InsetWell } from "@oikome/ui";
import type { NodeInfo } from "@oikome/client";
import { DataType } from "apache-arrow";
import type { Field, Table } from "apache-arrow";
import { useAction, useClient, useLoad } from "./data";
import { cell } from "./format";
import { strings } from "./strings";

/** A value of a column as text: dates as 2026-10-28, times in UTC. */
function show(field: Field, value: unknown): string | null {
  if (value === null || value === undefined) return null;
  if (DataType.isDate(field.type)) return new Date(Number(value)).toISOString().slice(0, 10);
  if (DataType.isTimestamp(field.type)) {
    return new Date(Number(value)).toISOString().replace("T", " ").slice(0, 19);
  }
  return cell(value);
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
              <th key={f.name} scope="col">
                {f.name}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr key={i}>
              {row.map((value, j) => (
                <td key={j}>
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
}: {
  doneLabel: string;
  onDone: (info: NodeInfo) => void;
  onLater?: () => void;
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
        {key.data && (
          <Button
            icon="download"
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
      </InsetWell>
      <Checkbox label={strings.recovery.stored} checked={stored} onChange={setStored} />
      {confirm.error && <ErrorText>{confirm.error}</ErrorText>}
      <div className="screen-actions">
        <Button
          variant="primary"
          large
          disabled={!stored || !key.data || confirm.busy}
          onClick={async () => {
            const info = await confirm.run(() => client.confirmRecoveryKey());
            if (info) onDone(info);
          }}
        >
          {doneLabel}
        </Button>
        {onLater && (
          <Button variant="text" onClick={onLater}>
            {strings.recovery.later}
          </Button>
        )}
      </div>
    </div>
  );
}
