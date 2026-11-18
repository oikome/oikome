// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Knowledge an AI can read (docs/knowledge-schema.md, PR 28 K1–K2):
// Prepare, where each original is, its passages, search by meaning with a
// model that runs on this computer, and a retrieval test that searches the
// way an assistant does.

import { Fragment, useEffect, useState } from "react";
import type { FormEvent, ReactNode } from "react";
import {
  Button,
  Card,
  ChoiceList,
  ConfirmDialog,
  Dialog,
  EmptyState,
  ErrorText,
  Group,
  Icon,
  Row,
  Segmented,
  Status,
  TextField,
} from "@oikome/ui";
import type { StatusKind } from "@oikome/ui";
import type {
  EmbeddingModel,
  KnowledgeChunk,
  KnowledgeFile,
  KnowledgeStage,
  KnowledgeStatus,
  RetrievalResults,
  SearchMode,
} from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { bytes } from "../format";
import { strings } from "../strings";

const k = strings.knowledge;

const KINDS: Record<KnowledgeStage, StatusKind> = {
  waiting: "idle",
  reading: "run",
  splitting: "run",
  embedding: "run",
  ready: "ok",
  failed: "bad",
};

function stageText(file: KnowledgeFile): string {
  const stage = k.stages[file.stage];
  const reason = file.reason ? k.reasons[file.reason] : undefined;
  return reason ? `${stage}: ${reason}` : stage;
}

/** Where a passage is in its original: headings, page, lines. */
function where(chunk: KnowledgeChunk): string {
  const parts = [...chunk.heading];
  if (chunk.page != null) parts.push(k.page(chunk.page));
  parts.push(k.lines(chunk.lineStart, chunk.lineEnd));
  return parts.join(" › ");
}

/** The embedding models, looked at again while one downloads. */
function useModels() {
  const client = useClient();
  const models = useLoad(() => client.listModels(), []);
  const downloading = models.data?.some((m) => m.state === "downloading") ?? false;
  useEffect(() => {
    if (!downloading) return;
    const timer = setTimeout(models.reload, 1500);
    return () => clearTimeout(timer);
  }, [models.data, downloading, models.reload]);
  return models;
}

/** A model's download: the button, how far it is, or that it is ready. */
function ModelDownload({ model, onChange }: { model: EmbeddingModel; onChange: () => void }) {
  const client = useClient();
  const download = useAction();
  const start = async () => {
    if (await download.run(() => client.downloadModel({ id: model.id }))) onChange();
  };
  if (model.state === "ready") return <p className="screen-help">{k.modelReady}</p>;
  if (model.state === "downloading") {
    const pct = model.size ? Math.floor((model.done / model.size) * 100) : 0;
    return (
      <div className="screen-model-progress" role="status">
        <progress max={100} value={pct} aria-label={model.name} />
        <span>{k.modelDownloading(pct)}</span>
      </div>
    );
  }
  return (
    <div className="screen-knowledge-install">
      <p>{model.state === "failed" ? k.modelFailed : k.searchMeaningHelp(model.name, bytes(model.size))}</p>
      <Button icon="download" disabled={download.busy} onClick={() => void start()}>
        {k.modelDownload}
      </Button>
      {download.error && <ErrorText>{download.error}</ErrorText>}
    </div>
  );
}

function Passage({ chunk, aside }: { chunk: KnowledgeChunk; aside?: ReactNode }) {
  return (
    <div className={chunk.enabled ? "screen-passage" : "screen-passage is-off"}>
      <div className="screen-passage__head">
        <span className="screen-passage__where" translate="no">
          {where(chunk)}
        </span>
        {aside}
      </div>
      <p className="screen-passage__text">{chunk.text}</p>
    </div>
  );
}

/** Prepare a collection for AI: the format, explained and chosen; Lance
 * support when this computer lacks it; the first passages it will make. */
export function KnowledgeDialog({
  space,
  onClose,
  onPrepared,
}: {
  space: string;
  onClose: () => void;
  onPrepared: () => void;
}) {
  const client = useClient();
  const status = useLoad(() => client.knowledgeStatus({ space }), [space]);
  const preview = useLoad(() => client.knowledgePreview({ space }), [space]);
  const install = useAction();
  const prepare = useAction();
  const models = useModels();
  const [format, setFormat] = useState("lance");
  const [search, setSearch] = useState<"words" | "meaning">("words");
  const [installed, setInstalled] = useState(false);
  const available = status.data?.available ?? false;
  const model = models.data?.[0];
  const ready = search === "words" || model?.state === "ready";

  const doInstall = async () => {
    if (await install.run(() => client.installLance())) {
      setInstalled(true);
      status.reload();
    }
  };
  const doPrepare = async () => {
    const done = await prepare.run(async () => {
      await client.setKnowledgeModel({ space, model: search === "meaning" && model ? model.id : null });
      return client.prepareKnowledge({ space });
    });
    if (done) {
      onPrepared();
      onClose();
    }
  };

  return (
    <Dialog
      title={k.title(space)}
      closeLabel={strings.common.close}
      onClose={onClose}
      placement="top"
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button
            variant="primary"
            disabled={!available || format !== "lance" || !ready || prepare.busy}
            onClick={() => void doPrepare()}
          >
            {prepare.busy ? k.preparing : k.action}
          </Button>
        </>
      }
    >
      <p className="oik-dialog__text">{k.body}</p>
      <ChoiceList
        label={k.formatLabel}
        name="knowledge-format"
        value={format}
        onChange={setFormat}
        choices={[
          { value: "lance", title: k.formats.lance, meta: k.formats.lanceHelp, leading: <Icon name="ai" /> },
          { value: "iceberg", title: k.formats.iceberg, meta: k.formats.icebergHelp, leading: <Icon name="table" /> },
        ]}
      />
      {model && (
        <ChoiceList
          label={k.searchLabel}
          name="knowledge-search"
          value={search}
          onChange={(value) => setSearch(value === "meaning" ? "meaning" : "words")}
          choices={[
            { value: "words", title: k.searchWords, meta: k.searchWordsHelp },
            {
              value: "meaning",
              title: k.searchMeaning,
              meta: k.searchMeaningHelp(model.name, bytes(model.size)),
              more: model.state === "ready" ? undefined : <ModelDownload model={model} onChange={models.reload} />,
            },
          ]}
        />
      )}
      {installed && available && <p className="screen-help screen-knowledge-ready">{k.installed}</p>}
      {status.data && !available && (
        <div className="screen-knowledge-install">
          <p>{k.install}</p>
          <Button icon="download" disabled={install.busy} onClick={() => void doInstall()}>
            {install.busy ? k.installing : k.installAction}
          </Button>
        </div>
      )}
      <section className="screen-knowledge-preview" aria-label={k.preview}>
        <h3 className="oik-label">{k.preview}</h3>
        {preview.data && preview.data.length === 0 && <p className="screen-help">{k.previewNone}</p>}
        {preview.data?.map((chunk, i) => <Passage key={i} chunk={chunk} />)}
      </section>
      {(status.error || preview.error) && <ErrorText>{status.error ?? preview.error}</ErrorText>}
      {install.error && <ErrorText>{install.error}</ErrorText>}
      {prepare.error && <ErrorText>{prepare.error}</ErrorText>}
    </Dialog>
  );
}

function Chunks({ space, path }: { space: string; path: string }) {
  const client = useClient();
  const chunks = useLoad(() => client.knowledgeChunks({ space, path }), [space, path]);
  const toggle = useAction();
  const flip = async (chunk: KnowledgeChunk) => {
    const done = await toggle.run(() =>
      client.setKnowledgeChunk({ space, chunkId: chunk.chunkId, enabled: !chunk.enabled }),
    );
    if (done) chunks.reload();
  };
  return (
    <div className="screen-knowledge-chunks">
      {chunks.data?.map((chunk) => (
        <Passage
          key={chunk.chunkId}
          chunk={chunk}
          aside={
            <Button variant="quiet" disabled={toggle.busy} onClick={() => void flip(chunk)}>
              {chunk.enabled ? k.disable : k.enable}
            </Button>
          }
        />
      ))}
      {(chunks.error || toggle.error) && <ErrorText>{chunks.error ?? toggle.error}</ErrorText>}
    </div>
  );
}

function RetrievalTest({ space, meaning }: { space: string; meaning: boolean }) {
  const client = useClient();
  const search = useAction();
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState<SearchMode>("hybrid");
  const [results, setResults] = useState<RetrievalResults>();
  const run = async (how: SearchMode) => {
    if (!query.trim()) return;
    const found = await search.run(() =>
      client.retrievalTest({ space, query: query.trim(), limit: 10, mode: meaning ? how : "words" }),
    );
    if (found) setResults(found);
  };
  const submit = (e: FormEvent) => {
    e.preventDefault();
    void run(mode);
  };
  return (
    <Group
      title={k.test}
      action={
        meaning ? (
          <Segmented
            label={k.searchLabel}
            value={mode}
            onChange={(value) => {
              const how: SearchMode = value === "words" || value === "meaning" ? value : "hybrid";
              setMode(how);
              if (results) void run(how);
            }}
            options={[
              { value: "hybrid", label: k.modes.hybrid },
              { value: "words", label: k.modes.words },
              { value: "meaning", label: k.modes.meaning },
            ]}
          />
        ) : undefined
      }
      footnote={k.testHelp}
    >
      <li className="oik-group__item">
        <form className="screen-retrieval" onSubmit={submit}>
          <TextField name="retrieval-query" label={k.testPlaceholder} value={query} onChange={setQuery} />
          <Button type="submit" variant="primary" disabled={!query.trim() || search.busy}>
            {k.testAction}
          </Button>
        </form>
        {search.error && <ErrorText>{search.error}</ErrorText>}
        {results && (
          <div className="screen-retrieval__results" aria-live="polite">
            <p className="screen-help">
              {results.hits.length ? k.testResult(results.hits.length, results.tookMs) : k.testNone}
            </p>
            <ol className="screen-retrieval__list">
              {results.hits.map((hit, i) => (
                <li key={hit.chunkId}>
                  <Passage
                    chunk={hit}
                    aside={
                      <span className="screen-retrieval__rank">
                        <span translate="no">{`#${i + 1} ${hit.path}`}</span>
                        {hit.foundBy && <span>{k.foundBy[hit.foundBy]}</span>}
                        {hit.score != null && (
                          <span>{k.score(hit.score.toFixed(results.mode === "hybrid" ? 3 : 2))}</span>
                        )}
                      </span>
                    }
                  />
                </li>
              ))}
            </ol>
          </div>
        )}
      </li>
    </Group>
  );
}

/** Whether a collection is searched by meaning, and turning it on or off. */
function MeaningSetting({ status, onChange }: { status: KnowledgeStatus; onChange: () => void }) {
  const client = useClient();
  const models = useModels();
  const change = useAction();
  const model = models.data?.find((m) => m.id === status.model) ?? models.data?.[0];
  if (!model) return null;
  const set = async (on: boolean) => {
    const done = await change.run(() => client.setKnowledgeModel({ space: status.space, model: on ? model.id : null }));
    if (done) onChange();
  };
  const meta = !status.model
    ? k.meaningOff
    : !status.meaning
      ? k.meaningNoModel(model.name)
      : status.files.some((f) => f.stage === "embedding" || f.stage === "waiting")
        ? k.meaningPending
        : k.meaningOn(model.name);
  return (
    <Group title={k.searchLabel}>
      <Row
        title={k.meaning}
        meta={meta}
        trailing={
          <Button
            variant={status.model ? "quiet" : "secondary"}
            disabled={change.busy || (!status.model && model.state !== "ready")}
            onClick={() => void set(!status.model)}
          >
            {status.model ? k.turnOff : k.turnOn}
          </Button>
        }
      />
      {model.state !== "ready" && (
        <li className="oik-group__item screen-model-row">
          <ModelDownload model={model} onChange={models.reload} />
        </li>
      )}
      {change.error && (
        <li className="oik-group__item screen-model-row">
          <ErrorText>{change.error}</ErrorText>
        </li>
      )}
    </Group>
  );
}

/** Settings › AI Models: each model, its size and license, what uses it,
 * and downloading or removing it. */
export function ModelsSettings() {
  const client = useClient();
  const models = useModels();
  const remove = useAction();
  const [removing, setRemoving] = useState<EmbeddingModel>();
  if (models.error) return <Group title={k.modelsTitle}><Row title={models.error} muted /></Group>;
  if (!models.data) return null;
  return (
    <Group title={k.modelsTitle} footnote={k.modelsFootnote}>
      {models.data.map((model) => (
        <Fragment key={model.id}>
          <Row
            title={<span translate="no">{model.name}</span>}
            meta={
              model.usedBy.length
                ? `${k.modelMeta(bytes(model.size), model.license)} · ${k.modelUsedBy(model.usedBy.join(", "))}`
                : k.modelMeta(bytes(model.size), model.license)
            }
            trailing={
              model.state === "ready" ? (
                <Button variant="quiet" onClick={() => setRemoving(model)}>
                  {k.modelRemove}
                </Button>
              ) : undefined
            }
          />
          {model.state !== "ready" && (
            <li className="oik-group__item screen-model-row">
              <ModelDownload model={model} onChange={models.reload} />
            </li>
          )}
        </Fragment>
      ))}
      {removing && (
        <ConfirmDialog
          title={k.modelRemoveTitle(removing.name)}
          body={k.modelRemoveBody}
          confirmLabel={k.modelRemove}
          cancelLabel={strings.common.cancel}
          busy={remove.busy}
          error={remove.error}
          onConfirm={async () => {
            if (await remove.run(() => client.removeModel({ id: removing.id }))) {
              setRemoving(undefined);
              models.reload();
            }
          }}
          onCancel={() => {
            setRemoving(undefined);
            remove.clear();
          }}
        />
      )}
    </Group>
  );
}

/** A prepared collection's knowledge: how it is searched, the retrieval
 * test, and each original with its passages. */
export function KnowledgeView({ space }: { space: string }) {
  const client = useClient();
  const status = useLoad(() => client.knowledgeStatus({ space }), [space]);
  const retry = useAction();
  const [open, setOpen] = useState<string>();
  const [preparing, setPreparing] = useState(false);
  const data = status.data;

  // While originals are being prepared, look again every two seconds.
  useEffect(() => {
    if (!data || data.waiting === 0) return;
    const timer = setTimeout(status.reload, 2000);
    return () => clearTimeout(timer);
  }, [data, status.reload]);

  const doRetry = async () => {
    if (await retry.run(() => client.retryKnowledge({ space }))) status.reload();
  };

  if (status.error) return <ErrorText>{status.error}</ErrorText>;
  if (!data) return <div className="screen-page-skeleton oik-skel" aria-hidden="true" />;
  if (data.preparedMs == null) {
    return (
      <Card>
        <EmptyState
          icon="ai"
          title={k.emptyTitle}
          body={k.emptyBody}
          action={
            <Button variant="primary" onClick={() => setPreparing(true)}>
              {k.prepare}
            </Button>
          }
        />
        {preparing && <KnowledgeDialog space={space} onClose={() => setPreparing(false)} onPrepared={status.reload} />}
      </Card>
    );
  }

  return (
    <div className="screen-knowledge">
      <MeaningSetting status={data} onChange={status.reload} />
      <RetrievalTest space={space} meaning={data.meaning} />
      <Group
        title={k.passages}
        action={
          data.failed > 0 ? (
            <Button variant="quiet" disabled={retry.busy} onClick={() => void doRetry()}>
              {k.retry}
            </Button>
          ) : undefined
        }
        footnote={k.summary(data.ready, data.waiting, data.failed, data.chunks)}
      >
        {data.files.map((file) => (
          <Fragment key={file.path}>
            <Row
              title={<span translate="no">{file.path}</span>}
              meta={k.chunks(file.chunks)}
              trailing={<Status kind={KINDS[file.stage]}>{stageText(file)}</Status>}
              onClick={file.chunks > 0 ? () => setOpen(open === file.path ? undefined : file.path) : undefined}
              label={file.chunks > 0 ? `${open === file.path ? k.hideChunks : k.showChunks}: ${file.path}` : undefined}
              chevron={file.chunks > 0}
            />
            {open === file.path && (
              <li className="oik-group__item">
                <Chunks space={space} path={file.path} />
              </li>
            )}
          </Fragment>
        ))}
      </Group>
      {retry.error && <ErrorText>{retry.error}</ErrorText>}
    </div>
  );
}
