// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { useMemo, useState } from "react";
import { AgGridReact } from "ag-grid-react";
import { ClientSideRowModelModule, ModuleRegistry, themeQuartz } from "ag-grid-community";
import type { CellFocusedEvent, ColDef, ValueFormatterParams } from "ag-grid-community";
import { DataType } from "apache-arrow";
import type { Table } from "apache-arrow";
import { Button } from "@oikome/ui";
import { columnLabel } from "./format";
import { locale } from "./i18n/languages";
import { columnDecimals, numeric, show } from "./parts";
import { strings } from "./strings";

ModuleRegistry.registerModules([ClientSideRowModelModule]);

const theme = themeQuartz.withParams({
  backgroundColor: "var(--bg)",
  foregroundColor: "var(--ink)",
  headerBackgroundColor: "var(--bg)",
  oddRowBackgroundColor: "var(--bg)",
  borderColor: "var(--line2)",
  accentColor: "var(--ink)",
  fontFamily: "var(--font)",
  fontSize: 13,
  headerFontSize: 11,
  rowHeight: 38,
});

type GridRow = Record<string, unknown> & { __row: number };

export default function DataGrid({ table, label }: { table: Table; label: string }) {
  const [copied, setCopied] = useState(false);
  const [focused, setFocused] = useState<string>();
  const fields = table.schema.fields;
  const rows = useMemo<GridRow[]>(
    () =>
      Array.from({ length: table.numRows }, (_, row) => ({
        __row: row,
        ...Object.fromEntries(fields.map((field, column) => [field.name, table.getChildAt(column)?.get(row) as unknown])),
      })),
    [table, fields],
  );
  const columns = useMemo<ColDef<GridRow>[]>(
    () =>
      fields.map((field, column) => {
        const digits = DataType.isFloat(field.type) ? columnDecimals(table, column) : 0;
        const float = DataType.isFloat(field.type)
          ? new Intl.NumberFormat(locale(), { minimumFractionDigits: digits, maximumFractionDigits: digits })
          : undefined;
        const present = (value: unknown) =>
          float && typeof value === "number" && Number.isFinite(value) ? float.format(value) : show(field, value);
        let longest = columnLabel(field.name).length;
        for (let row = 0; row < table.numRows; row++) {
          longest = Math.max(longest, (present(table.getChildAt(column)?.get(row) as unknown) ?? "").length);
        }
        return {
          field: field.name,
          headerName: columnLabel(field.name),
          width: Math.min(360, Math.max(112, longest * 8 + 28)),
          minWidth: 96,
          resizable: true,
          sortable: false,
          wrapText: true,
          autoHeight: true,
          cellClass: numeric(field) ? "screen-data-grid__number" : undefined,
          valueFormatter: (params: ValueFormatterParams<GridRow>) => present(params.value as unknown) ?? "—",
          tooltipValueGetter: (params) => show(field, params.value as unknown),
        };
      }),
    [table, fields],
  );

  const focus = (event: CellFocusedEvent<GridRow>) => {
    const id = typeof event.column === "string" ? event.column : event.column?.getColId();
    const column = fields.findIndex((item) => item.name === id);
    const field = fields[column];
    const value = event.rowIndex === null || event.rowIndex === undefined ? undefined : table.getChildAt(column)?.get(event.rowIndex);
    setFocused(field ? (show(field, value as unknown) ?? "—") : undefined);
    setCopied(false);
  };

  return (
    <div className="screen-data-grid" role="region" aria-label={label}>
      <div className="screen-data-grid__bar">
        <Button
          variant="quiet"
          disabled={focused === undefined}
          onClick={async () => {
            if (focused === undefined) return;
            try {
              await navigator.clipboard.writeText(focused);
              setCopied(true);
            } catch {
              // The selected cell stays visible to copy by hand.
            }
          }}
        >
          {copied ? strings.common.copied : strings.common.copy}
        </Button>
      </div>
      <div className="screen-data-grid__grid">
        <AgGridReact<GridRow>
          theme={theme}
          rowData={rows}
          columnDefs={columns}
          getRowId={(params) => String(params.data.__row)}
          onCellFocused={focus}
          defaultColDef={{ suppressMovable: true }}
          headerHeight={36}
          suppressRowHoverHighlight={false}
          enableCellTextSelection
          ensureDomOrder
        />
      </div>
      <div className="screen-data-grid__records">
        {rows.map((row) => (
          <dl key={row.__row}>
            {fields.map((field) => (
              <div key={field.name}>
                <dt>{columnLabel(field.name)}</dt>
                <dd>{show(field, row[field.name]) ?? "—"}</dd>
              </div>
            ))}
          </dl>
        ))}
      </div>
    </div>
  );
}
