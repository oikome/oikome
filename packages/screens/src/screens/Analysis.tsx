// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Analysis: the analysis tables a folder collection's data files make
// (each file, or each folder of them, read as one table) when the owner
// chooses them, what each column is for, and each table's explorer: its
// charts (Explorer.tsx). The tables are the collection's, queried as
// <collection>.<table> by SQL and assistants; the files are only read, and
// the owner may remove a table. A dataset in its folders (a Lance dataset,
// an Iceberg table) is a table too, always, read where it is.

import { useEffect, useRef, useState } from "react";
import {
  Button,
  Card,
  Checkbox,
  ConfirmDialog,
  Dialog,
  EmptyState,
  ErrorText,
  Group,
  Row,
  Segmented,
  Select,
  Status,
  useToast,
} from "@oikome/ui";
import type { AnalysisSource, AnalysisTable, ColumnType, DatasetFacts, FileAnalysis } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { homeRelative } from "../format";
import { TransferProgress } from "../progress";
import { since } from "./Sources";
import { strings } from "../strings";
import { Explorer } from "./Explorer";

const a = strings.analysisView;

/**
 * The types a column can be read as, each the DuckDB type it is (as BI
 * tools offer them: a whole number, a floating one, an exact one for
 * money), and the one DuckDB read it as, if it is one of them.
 */
const READ_AS: ColumnType[] = ["text", "integer", "number", "decimal", "date", "timestamp", "boolean"];
/** The most digits a decimal has in DuckDB. */
const MAX_DIGITS = 38;
/** A decimal's digits in all and after the point, of a type written `DECIMAL(18, 4)` or `Decimal128(18, 4)`. */
type DecimalSize = [precision: number, scale: number];
const sizeOf = (type: string | undefined): DecimalSize | undefined => {
  const found = /^Decimal(?:128|256)?\((\d+), (\d+)\)$/i.exec(type ?? "");
  return found ? [Number(found[1]), Number(found[2])] : undefined;
};
function kindOf(dataType: string): ColumnType | undefined {
  const exact: Partial<Record<string, ColumnType>> = {
    Utf8: "text",
    LargeUtf8: "text",
    Utf8View: "text",
    Int64: "integer",
    Float64: "number",

    Date32: "date",
    Boolean: "boolean",
  };
  if (exact[dataType]) return exact[dataType];
  // Any whole number, floating one or timestamp, however wide or precise
  // (an Iceberg table's are Int32 and Timestamp(µs, None)).
  if (/^U?Int(8|16|32|64)$/.test(dataType)) return "integer";
  if (/^Float(16|32|64)$/.test(dataType)) return "number";
  if (/^Timestamp\(/.test(dataType)) return "timestamp";
  if (/^Decimal(128|256)\(/.test(dataType)) return "decimal";
  return undefined;
}
/** A type to read a column as, in words. */
const kindName = (kind: ColumnType) => a.readAsTypes[kind] ?? kind;
/**
 * What a column of a type can be read as, as the node allows: text as
 * anything, a number as text or another number, a date or a time as text or
 * another of them, true or false as text, itself or a whole number.
 */
function readableAs(dataType: string): ColumnType[] {
  if (/^(Utf8|LargeUtf8|Utf8View)$/.test(dataType)) return READ_AS;
  if (/^(U?Int|Float|Decimal)/.test(dataType)) return ["text", "integer", "number", "decimal"];
  if (/^(Date|Timestamp)/.test(dataType)) return ["text", "date", "timestamp"];
  if (dataType === "Boolean") return ["text", "integer", "boolean"];
  return ["text"];
}

/** A column's type as DuckDB names it, from the Arrow type it was read as. */
function duckType(dataType: string): string {
  const named: Record<string, string> = {
    Utf8: "VARCHAR",
    LargeUtf8: "VARCHAR",
    Utf8View: "VARCHAR",
    Boolean: "BOOLEAN",
    Int8: "TINYINT",
    Int16: "SMALLINT",
    Int32: "INTEGER",
    Int64: "BIGINT",
    UInt8: "UTINYINT",
    UInt16: "USMALLINT",
    UInt32: "UINTEGER",
    UInt64: "UBIGINT",
    Float32: "FLOAT",
    Float64: "DOUBLE",
    Date32: "DATE",
    Date64: "DATE",
    Binary: "BLOB",
    LargeBinary: "BLOB",
  };
  if (dataType in named) return named[dataType]!;
  if (dataType.startsWith("Timestamp")) return dataType.includes("Some") ? "TIMESTAMP WITH TIME ZONE" : "TIMESTAMP";
  const decimal = /^Decimal(?:128|256)\((\d+), (\d+)\)$/.exec(dataType);
  if (decimal) return `DECIMAL(${decimal[1]}, ${decimal[2]})`;
  return dataType;
}

/** Where a file's table comes from, how much it holds and when it was made, in a sentence. */
function aboutFiles(table: AnalysisTable): string {
  const first = table.files[0] ?? "";
  const folder = first.includes("/") ? first.slice(0, first.lastIndexOf("/")) : "";
  const source = table.sheet
    ? a.inSheet(first, table.sheet)
    : table.files.length === 1
      ? first
      : folder
        ? a.filesIn(folder, table.files.length)
        : a.files(table.files.length);
  return a.aboutFiles(source, a.rows(table.rows), since(table.builtAtMs));
}

/** What a dataset is, how much it holds and when it was written, in a sentence. */
function aboutDataset(dataset: DatasetFacts, rows: number | undefined): string {
  const kind = a.datasetKinds[dataset.kind] ?? dataset.kind;
  const held = rows === undefined ? undefined : a.rows(rows);
  return a.aboutDataset(kind, held, dataset.writtenAtMs ? since(dataset.writtenAtMs) : undefined);
}

/** One analysis table: its facts, its columns and what each is for, and its totals. */
/** What this computer lacks to read a dataset: DuckDB's extension, its
 * download offered and followed. */
function DatasetReader({ dataset, onReady }: { dataset: DatasetFacts; onReady: () => void }) {
  const client = useClient();
  const install = useAction();
  const extension = dataset.extension;
  const going = extension?.state === "downloading" || extension?.state === "installing";
  const failed = extension?.state === "failed";
  return (
    <Card className="screen-analysis__reader">
      <p>{a.needsExtension(dataset.kind)}</p>
      {going && extension ? (
        <TransferProgress
          done={extension.done}
          total={extension.total}
          label={a.downloadLabel(dataset.kind)}
          installing={extension.state === "installing"}
          note={strings.knowledge.download.savedTo(homeRelative(extension.path))}
        />
      ) : (
        <Button
          variant="primary"
          disabled={install.busy}
          onClick={async () => {
            if (await install.run(() => client.installExtension({ name: dataset.kind }))) onReady();
          }}
        >
          {failed ? a.downloadAgain : a.download}
        </Button>
      )}
      {failed && extension?.error && <ErrorText>{extension.error}</ErrorText>}
      {install.error && <ErrorText>{install.error}</ErrorText>}
    </Card>
  );
}

/** Where a table not made yet comes from: its file, its folder's files,
 * or its sheet (`inBook`: under its spreadsheet, which is shown above). */
function sourceAt(source: AnalysisSource, inBook = false): string {
  const first = source.files[0] ?? "";
  if (source.sheet) return inBook ? a.sheet(source.sheet) : a.inSheet(first, source.sheet);
  if (source.files.length === 1) return first;
  const folder = first.includes("/") ? first.slice(0, first.lastIndexOf("/")) : "";
  return folder ? a.filesIn(folder, source.files.length) : a.files(source.files.length);
}

/** The tables not made yet, a spreadsheet's several sheets together under
 * it; the rest one by one, in the order given. */
function grouped(sources: AnalysisSource[]): { book?: string; sources: AnalysisSource[] }[] {
  const out: { book?: string; sources: AnalysisSource[] }[] = [];
  const books = new Map<string, AnalysisSource[]>();
  for (const source of sources) {
    const book = source.sheet ? source.files[0] : undefined;
    if (book === undefined) {
      out.push({ sources: [source] });
      continue;
    }
    const sheets = books.get(book);
    if (sheets) sheets.push(source);
    else {
      const fresh = [source];
      books.set(book, fresh);
      out.push({ book, sources: fresh });
    }
  }
  // A spreadsheet of one sheet is a row like a file's.
  return out.map((group) => (group.book && group.sources.length === 1 ? { sources: group.sources } : group));
}

/** Which of a folder collection's files to analyse: each item makes one
 * table, none chosen at first. A spreadsheet's sheets are chosen one by one
 * or together. */
function MakeTables({
  space,
  sources,
  onMade,
  onClose,
}: {
  space: string;
  sources: AnalysisSource[];
  onMade: (made: FileAnalysis, chosen: string[]) => void;
  onClose: () => void;
}) {
  const client = useClient();
  const making = useAction();
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const all = sources.length > 0 && chosen.size === sources.length;
  const choose = (names: string[], on: boolean) => {
    const next = new Set(chosen);
    for (const name of names) {
      if (on) next.add(name);
      else next.delete(name);
    }
    setChosen(next);
    making.clear();
  };
  const make = async () => {
    const names = sources.map((source) => source.name).filter((name) => chosen.has(name));
    const made = await making.run(() => client.buildAnalysis({ space, tables: names }));
    if (made) onMade(made, names);
  };
  const item = (source: AnalysisSource, inBook = false) => (
    <li key={source.name}>
      <Checkbox
        label={
          <span className="screen-analysis-pick__item">
            <span className="oik-mono" translate="no">
              {source.name}
            </span>
            <span className="screen-help" translate="no" title={source.files.join("\n")}>
              {sourceAt(source, inBook)}
            </span>
          </span>
        }
        checked={chosen.has(source.name)}
        disabled={making.busy}
        onChange={(on) => choose([source.name], on)}
      />
    </li>
  );

  return (
    <Dialog
      title={a.pickTitle}
      closeLabel={strings.common.close}
      onClose={onClose}
      placement="top"
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button variant="primary" disabled={chosen.size === 0 || making.busy} onClick={() => void make()}>
            {making.busy ? a.making : a.makeN(chosen.size)}
          </Button>
        </>
      }
    >
      <p className="oik-dialog__text">{a.pickBody}</p>
      {sources.length > 1 && (
        <div className="screen-save-all">
          <Checkbox
            label={a.selectAll}
            checked={all}
            indeterminate={chosen.size > 0 && !all}
            disabled={making.busy}
            onChange={(on) => choose(sources.map((source) => source.name), on)}
          />
          <span className="screen-save-all__count">{a.chosen(chosen.size, sources.length)}</span>
        </div>
      )}
      <ul className="screen-analysis-pick" aria-label={a.pickTitle}>
        {grouped(sources).map((group) => {
          if (!group.book) return item(group.sources[0]!);
          const names = group.sources.map((source) => source.name);
          const some = names.filter((name) => chosen.has(name)).length;
          return (
            <li key={`book:${group.book}`}>
              <Checkbox
                label={
                  <span className="screen-analysis-pick__item">
                    <span translate="no">{group.book}</span>
                  </span>
                }
                checked={some === names.length}
                indeterminate={some > 0 && some < names.length}
                disabled={making.busy}
                onChange={(on) => choose(names, on)}
              />
              <ul className="screen-analysis-pick screen-analysis-pick--sheets">
                {group.sources.map((source) => item(source, true))}
              </ul>
            </li>
          );
        })}
      </ul>
      {making.busy && (
        <span className="screen-progress" role="progressbar" aria-label={a.making}>
          <i />
        </span>
      )}
      {making.error && <ErrorText>{making.error}</ErrorText>}
    </Dialog>
  );
}

/** Remove an analysis table made of files, once asked: the table goes
 * with the roles set for its columns; its files stay as they are. */
function RemoveTable({
  space,
  table,
  onRemoved,
  onClose,
}: {
  space: string;
  table: AnalysisTable;
  onRemoved: (next: FileAnalysis) => void;
  onClose: () => void;
}) {
  const client = useClient();
  const toast = useToast();
  const removing = useAction();
  return (
    <ConfirmDialog
      title={a.removeTitle(table.name)}
      body={a.removeBody(`${space}.${table.name}`)}
      confirmLabel={removing.busy ? a.removing : a.removeYes}
      cancelLabel={strings.common.cancel}
      busy={removing.busy}
      error={removing.error}
      onCancel={onClose}
      onConfirm={async () => {
        const next = await removing.run(() => client.removeAnalysisTable({ space, table: table.name }));
        if (!next) return;
        toast(a.removed(table.name));
        onRemoved(next);
      }}
    />
  );
}

function TableView({
  space,
  table,
  onChange,
  onReload,
}: {
  space: string;
  table: AnalysisTable;
  onChange: (next: FileAnalysis) => void;
  onReload: () => void;
}) {
  const client = useClient();
  const setType = useAction();
  const [removing, setRemoving] = useState(false);
  const dataset = table.dataset;
  const readable = !table.problem;
  // While a reader downloads, look again every second.
  const downloading = dataset?.extension?.state === "downloading" || dataset?.extension?.state === "installing";
  useEffect(() => {
    if (!downloading) return;
    const timer = setTimeout(onReload, 1000);
    return () => clearTimeout(timer);
  }, [downloading, table, onReload]);
  return (
    <div className="screen-analysis__table">
      {dataset?.extension ? (
        <DatasetReader dataset={dataset} onReady={onReload} />
      ) : (
        table.problem && (
          <ErrorText>
            {dataset ? a.datasetProblem : a.problem} {table.problem}
          </ErrorText>
        )
      )}
      {/* What the table is, in one line: the name it is queried by, where it comes from, how much it holds. */}
      <div className="screen-analysis__about">
        <p className="screen-analysis__line">
          <code className="screen-analysis__query" translate="no">
            {space}.{table.name}
          </code>{" "}
          <span className="screen-help">{dataset ? aboutDataset(dataset, readable ? table.rows : undefined) : aboutFiles(table)}</span>
        </p>
        {!dataset && <Button onClick={() => setRemoving(true)}>{a.remove}</Button>}
      </div>
      {removing && (
        <RemoveTable
          space={space}
          table={table}
          onClose={() => setRemoving(false)}
          onRemoved={(next) => {
            setRemoving(false);
            onChange(next);
          }}
        />
      )}

      {/* Exploring needs the rows: nothing to explore while the table can't be read. */}
      {readable && <Explorer space={space} table={table} onChange={onChange} />}

      {table.fields.length > 0 && (
        <Group title={a.fields} footnote={a.fieldsNote}>
          {table.fields
            .filter((field) => field.name !== "_source_file")
            .map((field) => {
              // Read as DuckDB read it, or as the owner chose: charts read it so.
              const natural = kindOf(field.dataType);
              const shown = field.readAs ?? natural;
              const read = duckType(field.dataType);
              const choices = readableAs(field.dataType);
              const own = sizeOf(field.dataType);
              const [precision, scale] = sizeOf(field.readType) ?? own ?? [18, 2];
              // As DuckDB read it again when that is what is chosen; else read
              // as chosen: a decimal with the size chosen, or with its places
              // and as many digits as its values need.
              const choose = async (kind: ColumnType | undefined, size?: DecimalSize) => {
                const same = kind === natural && (kind !== "decimal" || !size || (own !== undefined && size[0] === own[0] && size[1] === own[1]));
                const readAs = kind === undefined || same ? undefined : kind;
                const decimal = readAs === "decimal" ? (size ? { precision: size[0], scale: size[1] } : { scale }) : {};
                const next = await setType.run(
                  () => client.setAnalysisType({ space, table: table.name, column: field.name, readAs, ...decimal }),
                  // The one refusal left: a value too large for the size.
                  (e) => (e.code === "invalid" && readAs === "decimal" ? a.decimalNoFit(field.name, `DECIMAL(${size?.[0] ?? MAX_DIGITS}, ${size?.[1] ?? scale})`) : undefined),
                );
                if (next) onChange(next);
                return next !== undefined;
              };
              return (
                <Row
                  key={field.name}
                  title={<span translate="no">{field.name}</span>}
                  meta={field.readAs && field.readType ? a.convertedFrom(field.readType, read) : read}
                  trailing={
                    // The type in one column in every row: a decimal's size comes before it.
                    <div className="screen-analysis__type-pick">
                      {shown === "decimal" && (
                        <DecimalDigits
                          column={field.name}
                          precision={precision}
                          scale={scale}
                          onApply={(size) => choose("decimal", size)}
                          onProblem={setType.fail}
                        />
                      )}
                      <Select
                        label={a.typeFor(field.name)}
                        hideLabel
                        value={shown ?? ""}
                        onChange={(value) => void choose(value === "" ? undefined : (value as ColumnType))}
                        options={[
                          ...(natural ? [] : [{ value: "", label: read }]),
                          ...READ_AS.filter((kind) => choices.includes(kind) || kind === natural).map((kind) => ({ value: kind, label: kindName(kind) })),
                        ]}
                      />
                    </div>
                  }
                />
              );
            })}
        </Group>
      )}
      {setType.error && <ErrorText>{setType.error}</ErrorText>}
    </div>
  );
}

/**
 * A decimal's size as two boxes: its digits in all (1 to 38) and after the
 * point (0 to its digits). A box applies what is typed when it is left or on
 * Enter; Escape, or a size refused, puts back the one in use.
 */
function DecimalDigits({
  column,
  precision,
  scale,
  onApply,
  onProblem,
}: {
  column: string;
  precision: number;
  scale: number;
  onApply: (size: DecimalSize) => Promise<boolean>;
  onProblem: (message: string) => void;
}) {
  const [digits, setDigits] = useState(String(precision));
  const [places, setPlaces] = useState(String(scale));
  // Each box follows its own number, so an answer to one leaves the other as typed.
  useEffect(() => setDigits(String(precision)), [precision]);
  useEffect(() => setPlaces(String(scale)), [scale]);
  const skip = useRef(false);
  const putBack = () => {
    setDigits(String(precision));
    setPlaces(String(scale));
  };
  const apply = async () => {
    if (skip.current) {
      skip.current = false;
      return;
    }
    const number = (typed: string) => (/^\d{1,2}$/.test(typed.trim()) ? Number(typed) : Number.NaN);
    const p = number(digits);
    const s = number(places);
    if (p === precision && s === scale) return;
    if (!(p >= 1 && p <= MAX_DIGITS)) {
      putBack();
      onProblem(a.precisionRange(MAX_DIGITS));
      return;
    }
    if (!(s >= 0 && s <= p)) {
      putBack();
      onProblem(a.scaleRange);
      return;
    }
    if (!(await onApply([p, s]))) putBack();
  };
  const box = (name: string, label: string, value: string, set: (value: string) => void) => (
    <label className="screen-analysis__digits">
      <span>{label}</span>
      <input
        className="oik-input"
        name={name}
        aria-label={name === "precision" ? a.precisionFor(column) : a.scaleFor(column)}
        inputMode="numeric"
        autoComplete="off"
        maxLength={2}
        value={value}
        onChange={(e) => set(e.target.value)}
        onFocus={(e) => e.currentTarget.select()}
        onBlur={() => void apply()}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") {
            skip.current = true;
            putBack();
            e.currentTarget.blur();
          }
        }}
      />
    </label>
  );
  return (
    <>
      {box("precision", a.precision, digits, setDigits)}
      {box("scale", a.scale, places, setPlaces)}
    </>
  );
}

export function AnalysisView({ space }: { space: string }) {
  const client = useClient();
  const analysis = useLoad(() => client.fileAnalysis({ space }), [space]);
  const [picked, setPicked] = useState<string>();
  const [picking, setPicking] = useState(false);
  const [fresh, setFresh] = useState<FileAnalysis>();
  const data = fresh ?? analysis.data;

  if (analysis.error) return <ErrorText>{analysis.error}</ErrorText>;
  if (!data) return <div className="screen-page-skeleton oik-skel" aria-hidden="true" />;

  // Datasets are tables as soon as they are found; files wait to be chosen.
  const offered = data.sources.filter((source) => !source.dataset);
  const pick = picking && (
    <MakeTables
      space={space}
      sources={offered}
      onClose={() => setPicking(false)}
      onMade={(made, chosen) => {
        setPicking(false);
        setFresh(made);
        setPicked(chosen.find((name) => made.tables.some((table) => table.name === name)));
      }}
    />
  );

  if (data.tables.length === 0) {
    return (
      <Card>
        <EmptyState
          icon="table"
          title={a.emptyTitle}
          body={offered.length > 0 ? a.emptyBody : a.noSources}
          action={
            offered.length > 0 ? (
              <Button variant="primary" onClick={() => setPicking(true)}>
                {a.make}
              </Button>
            ) : undefined
          }
        />
        {pick}
      </Card>
    );
  }

  const current = data.tables.find((t) => t.name === picked) ?? data.tables[0]!;
  return (
    <div className="screen-analysis">
      <div className="screen-analysis__head">
        {data.tables.length > 1 && (
          <Segmented
            label={a.tables}
            value={current.name}
            onChange={setPicked}
            options={data.tables.map((t) => ({ value: t.name, label: t.name }))}
          />
        )}
        {offered.length > 0 && (
          <span className="screen-analysis__more">
            <Status kind="idle">{a.notMade(offered.length)}</Status>
            <Button onClick={() => setPicking(true)}>{a.make}</Button>
          </span>
        )}
      </div>
      {pick}
      <TableView
        key={current.name}
        space={space}
        table={current}
        onChange={setFresh}
        onReload={() => {
          setFresh(undefined);
          analysis.reload();
        }}
      />
    </div>
  );
}
