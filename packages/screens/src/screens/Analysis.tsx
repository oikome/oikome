// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Analysis: the analysis tables a folder collection's data files make
// (each file, or each folder of them, read as one table), what each column
// is for, and the table's totals by month and by category. The tables are
// the collection's, queried as <collection>.<table> by SQL and assistants;
// the files are only read. A dataset in its folders (a Lance dataset, an
// Iceberg table) is a table too, read where it is, and can be copied into a
// collection as Oikome's own table.

import { useEffect, useState } from "react";
import { Button, Card, Dialog, EmptyState, ErrorText, Group, Row, Segmented, Select, Status, TextField, useToast } from "@oikome/ui";
import type { AnalysisPoint, AnalysisTable, DatasetFacts, FieldRole, FileAnalysis } from "@oikome/client";
import { forgetPreviews } from "../content";
import { useAction, useClient, useLoad } from "../data";
import { homeRelative } from "../format";
import { TransferProgress } from "../progress";
import { navigate } from "../router";
import { since } from "./Sources";
import { strings } from "../strings";
import { locale } from "../i18n/languages";

const a = strings.analysisView;
const NAME = /^[A-Za-z_-][A-Za-z0-9_-]{0,63}$/;
const NEW = "__new__";
const ROLES: FieldRole[] = ["time", "amount", "category", "title", "key", "other"];

/** A column's type in words: date, date and time, text, number, yes or no. */
function typeOf(dataType: string): string {
  if (/^Date/.test(dataType)) return a.types.date;
  if (/^Timestamp/.test(dataType)) return a.types.datetime;
  if (/Utf8|Binary/.test(dataType)) return a.types.text;
  if (/^(U?Int|Float|Decimal)/.test(dataType)) return a.types.number;
  if (dataType === "Boolean") return a.types.boolean;
  return a.types.other;
}

function number(value: number): string {
  return new Intl.NumberFormat(locale(), { maximumFractionDigits: 2 }).format(value);
}

/** Totals as bars: each with its label, its value and its length. */
function Bars({ points, label, vertical = false }: { points: AnalysisPoint[]; label: string; vertical?: boolean }) {
  const most = Math.max(...points.map((p) => Math.abs(p.value)), 0) || 1;
  return (
    <ul
      className={vertical ? "screen-bars screen-bars--columns" : "screen-bars"}
      aria-label={label}
      style={vertical ? { gridTemplateColumns: `repeat(${points.length}, minmax(0, 1fr))` } : undefined}
    >
      {points.map((point) => (
        <li key={point.label} className="screen-bars__item">
          <span className="screen-bars__label" translate="no">
            {point.label}
          </span>
          <span className="screen-bars__track" aria-hidden="true">
            <i
              style={
                vertical
                  ? { height: `${Math.max(2, (Math.abs(point.value) / most) * 100)}%` }
                  : { width: `${Math.max(1, (Math.abs(point.value) / most) * 100)}%` }
              }
            />
          </span>
          <span className="screen-bars__value">{number(point.value)}</span>
        </li>
      ))}
    </ul>
  );
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

/** A dataset's table into a collection as Oikome's own: an existing one
 * that holds tables of its kind, or a new one. */
function CopyToOikome({ space, table, onClose }: { space: string; table: AnalysisTable; onClose: () => void }) {
  const client = useClient();
  const toast = useToast();
  const copying = useAction();
  const spaces = useLoad(() => client.listSpaces(), []);
  const kind = table.dataset!.kind;
  const fits = (spaces.data ?? []).filter(
    (x) => x.kind === "analytical" && !x.readonly && !x.inPlace && x.format === kind,
  );
  const taken = new Set((spaces.data ?? []).map((x) => x.name));
  const base = table.name.replace(/_/g, "-");
  const free = [base, ...Array.from({ length: 98 }, (_, i) => `${base}-${i + 2}`)].find((x) => !taken.has(x)) ?? base;
  const [into, setInto] = useState(NEW);
  const [typed, setTyped] = useState<string>();
  const name = typed ?? free;
  const nameOk = into !== NEW || (NAME.test(name) && !taken.has(name));

  const copy = async () => {
    const done = await copying.run(() =>
      client.copyToOikome({ space, table: table.name, to: into === NEW ? name : into, new: into === NEW }),
    );
    if (!done) return;
    forgetPreviews();
    toast(a.copied(done.table, done.space));
    onClose();
    navigate(`/collections/${encodeURIComponent(done.space)}`);
  };

  return (
    <Dialog
      title={a.copyTitle(table.name)}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button variant="primary" disabled={!nameOk || copying.busy || !spaces.data} onClick={() => void copy()}>
            {copying.busy ? a.copying : a.copyYes}
          </Button>
        </>
      }
    >
      <p className="oik-dialog__text">{kind === "lance" ? a.copyLance : a.copyIceberg}</p>
      <Select
        label={a.copyTo}
        value={into}
        onChange={(value) => {
          setInto(value);
          copying.clear();
        }}
        options={[
          ...fits.map((x) => ({ value: x.name, label: x.name })),
          { value: NEW, label: strings.collections.newCollection },
        ]}
      />
      {into === NEW && (
        <>
          <TextField
            name="copied-collection-name"
            label={strings.collections.collectionNameLabel}
            value={name}
            mono
            onChange={(value) => {
              setTyped(value);
              copying.clear();
            }}
          />
          {taken.has(name) && <ErrorText>{strings.collections.nameTaken(name)}</ErrorText>}
        </>
      )}
      <p className="screen-help">{a.copyKeeps}</p>
      {copying.busy && (
        <span className="screen-progress" role="progressbar" aria-label={a.copying}>
          <i />
        </span>
      )}
      {copying.error && <ErrorText>{copying.error}</ErrorText>}
    </Dialog>
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
  const setRole = useAction();
  const [copying, setCopying] = useState(false);
  const dataset = table.dataset;
  const readable = !table.problem;
  const summary = useLoad(
    async () => (readable ? client.analysisSummary({ space, table: table.name }) : undefined),
    [space, table.name, readable, table.fields.map((f) => `${f.name}:${f.role}`).join(",")],
  );
  // While a reader downloads, look again every second.
  const downloading = dataset?.extension?.state === "downloading" || dataset?.extension?.state === "installing";
  useEffect(() => {
    if (!downloading) return;
    const timer = setTimeout(onReload, 1000);
    return () => clearTimeout(timer);
  }, [downloading, table, onReload]);
  const data = summary.data;
  const measure = data?.amount ? a.sumOf(data.amount) : a.countRows;
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
      {dataset && (
        <div className="screen-analysis__dataset">
          <p className="screen-help">{a.readInPlace}</p>
          <Button disabled={!readable} onClick={() => setCopying(true)}>
            {a.copy}
          </Button>
        </div>
      )}
      {copying && <CopyToOikome space={space} table={table} onClose={() => setCopying(false)} />}
      <dl className="oik-card screen-facts screen-analysis__facts">
        {dataset && (
          <div>
            <dt className="oik-label">{a.kindLabel}</dt>
            <dd>{a.datasetKinds[dataset.kind]}</dd>
          </div>
        )}
        {dataset?.version && (
          <div>
            <dt className="oik-label">{a.versionLabel}</dt>
            <dd translate="no">
              {dataset.writtenAtMs ? a.version(dataset.version, since(dataset.writtenAtMs)) : dataset.version}
            </dd>
          </div>
        )}
        {readable && (
          <div>
            <dt className="oik-label">{a.rowsLabel}</dt>
            <dd>{a.rows(table.rows)}</dd>
          </div>
        )}
        <div>
          <dt className="oik-label">{dataset ? a.folderLabel : a.filesLabel}</dt>
          <dd translate="no" title={table.files.join("\n")}>
            {table.sheet
              ? a.inSheet(table.files[0], table.sheet)
              : table.files.length === 1
                ? table.files[0]
                : a.files(table.files.length)}
          </dd>
        </div>
        <div>
          <dt className="oik-label">{dataset ? a.read : a.updated}</dt>
          <dd>{since(table.builtAtMs)}</dd>
        </div>
        <div>
          <dt className="oik-label">{a.queryAsLabel}</dt>
          <dd className="oik-mono" translate="no">
            {space}.{table.name}
          </dd>
        </div>
      </dl>

      {/* Totals need the rows: none while the table can't be read. */}
      {readable && (
        <div className="screen-analysis__charts">
          <Card className="screen-analysis__chart">
            <h3 className="oik-label">{a.byMonth}</h3>
            <p className="screen-help">{measure}</p>
            {summary.error ? (
              <ErrorText>{summary.error}</ErrorText>
            ) : !data ? (
              <div className="screen-analysis__skeleton oik-skel" aria-hidden="true" />
            ) : data.byMonth.length > 0 ? (
              <Bars points={data.byMonth} label={a.byMonth} vertical />
            ) : (
              <p className="screen-help">{a.needTime}</p>
            )}
          </Card>
          <Card className="screen-analysis__chart">
            <h3 className="oik-label">{a.byCategory}</h3>
            <p className="screen-help">{data?.category ? `${data.category} · ${measure}` : measure}</p>
            {summary.error ? null : !data ? (
              <div className="screen-analysis__skeleton oik-skel" aria-hidden="true" />
            ) : data.byCategory.length > 0 ? (
              <Bars points={data.byCategory} label={a.byCategory} />
            ) : (
              <p className="screen-help">{a.needCategory}</p>
            )}
          </Card>
        </div>
      )}

      {table.fields.length > 0 && (
      <Group title={a.fields} footnote={dataset ? a.datasetFieldsFootnote : a.fieldsFootnote}>
        {table.fields
          .filter((field) => field.name !== "_source_file")
          .map((field) => (
            <Row
              key={field.name}
              title={<span translate="no">{field.name}</span>}
              meta={field.inferred ? `${typeOf(field.dataType)} · ${a.proposed}` : typeOf(field.dataType)}
              trailing={
                <Select
                  label={a.roleFor(field.name)}
                  hideLabel
                  value={field.role}
                  onChange={async (value) => {
                    const next = await setRole.run(() =>
                      client.setAnalysisField({ space, table: table.name, field: field.name, role: value as FieldRole }),
                    );
                    if (next) onChange(next);
                  }}
                  options={ROLES.map((role) => ({ value: role, label: a.roles[role] }))}
                />
              }
            />
          ))}
      </Group>
      )}
      {setRole.error && <ErrorText>{setRole.error}</ErrorText>}
    </div>
  );
}

export function AnalysisView({ space }: { space: string }) {
  const client = useClient();
  const analysis = useLoad(() => client.fileAnalysis({ space }), [space]);
  const build = useAction();
  const [picked, setPicked] = useState<string>();
  const [fresh, setFresh] = useState<FileAnalysis>();
  const data = fresh ?? analysis.data;

  const make = async () => {
    const made = await build.run(() => client.buildAnalysis({ space }));
    if (made) setFresh(made);
  };

  if (analysis.error) return <ErrorText>{analysis.error}</ErrorText>;
  if (!data) return <div className="screen-page-skeleton oik-skel" aria-hidden="true" />;

  if (data.tables.length === 0) {
    return (
      <Card>
        <EmptyState
          icon="table"
          title={a.emptyTitle}
          body={data.sources.length > 0 ? a.emptyBody : a.noSources}
          action={
            data.sources.length > 0 ? (
              <Button variant="primary" disabled={build.busy} onClick={() => void make()}>
                {build.busy ? a.making : a.make}
              </Button>
            ) : undefined
          }
        />
        {data.sources.length > 0 && (
          <ul className="screen-analysis__sources" aria-label={a.willMake}>
            {data.sources.map((source) => (
              <li key={source.name}>
                <span className="oik-mono" translate="no">
                  {source.name}
                </span>
                <span className="screen-help" translate="no" title={source.files.join("\n")}>
                  {source.dataset
                    ? a.datasetAt(a.datasetKinds[source.dataset]!, source.files[0]!)
                    : source.sheet
                      ? a.inSheet(source.files[0], source.sheet)
                      : source.files.length === 1
                        ? source.files[0]
                        : a.files(source.files.length)}
                </span>
              </li>
            ))}
          </ul>
        )}
        {build.error && <ErrorText>{build.error}</ErrorText>}
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
        {data.sources.length > 0 && (
          <span className="screen-analysis__more">
            <Status kind="idle">{a.notMade(data.sources.length)}</Status>
            <Button disabled={build.busy} onClick={() => void make()}>
              {build.busy ? a.making : a.make}
            </Button>
          </span>
        )}
      </div>
      {build.error && <ErrorText>{build.error}</ErrorText>}
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
