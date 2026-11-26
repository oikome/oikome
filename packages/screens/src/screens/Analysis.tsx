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

import { useEffect, useState } from "react";
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
  Status,
  useToast,
} from "@oikome/ui";
import type { AnalysisSource, AnalysisTable, DatasetFacts, FileAnalysis } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { homeRelative } from "../format";
import { TransferProgress } from "../progress";
import { since } from "./Sources";
import { strings } from "../strings";
import { Explorer } from "./Explorer";

const a = strings.analysisView;

/** A column's type in words: date, date and time, text, number, yes or no. */
function typeOf(dataType: string): string {
  if (/^Date/.test(dataType)) return a.types.date;
  if (/^Timestamp/.test(dataType)) return a.types.datetime;
  if (/Utf8|Binary/.test(dataType)) return a.types.text;
  if (/^(U?Int|Float|Decimal)/.test(dataType)) return a.types.number;
  if (dataType === "Boolean") return a.types.boolean;
  // Any other type by the name DuckDB gives it, not a vague "other".
  return duckType(dataType);
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
        <Group title={a.fields} footnote={`${a.fieldsNote} ${strings.explorer.assistantsNote}`}>
          {table.fields
            .filter((field) => field.name !== "_source_file")
            .map((field) => (
              <Row
                key={field.name}
                title={<span translate="no">{field.name}</span>}
                meta={typeOf(field.dataType)}
                trailing={
                  <span className="screen-analysis__type" translate="no">
                    {duckType(field.dataType)}
                  </span>
                }
              />
            ))}
        </Group>
      )}
    </div>
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
