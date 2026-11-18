// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// A collection's index (docs/knowledge-schema.md, K1–K2): building it for
// a purpose (full-text search, or full-text and semantic search with a
// model that runs on this computer), where each original is, its passages,
// its skills, and a retrieval test that searches the way an assistant does.

import { Fragment, useEffect, useRef, useState } from "react";
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
import { bytes, homeRelative } from "../format";
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

/** A download in progress: how much of how much, how fast, about how
 * long is left, and where the files go. */
function DownloadProgress({
  done,
  total,
  path,
  label,
  installing,
}: {
  done: number;
  total: number | null;
  path: string;
  label: string;
  installing?: boolean;
}) {
  const [rate, setRate] = useState<number>();
  const last = useRef<{ at: number; done: number } | undefined>(undefined);
  useEffect(() => {
    const at = performance.now();
    const before = last.current;
    if (before && done > before.done && at > before.at) {
      const now = ((done - before.done) * 1000) / (at - before.at);
      setRate((old) => (old ? old * 0.6 + now * 0.4 : now));
    }
    last.current = { at, done };
  }, [done]);
  const pct = total ? Math.min(100, Math.floor((done / total) * 100)) : undefined;
  const parts: string[] = [];
  if (installing) {
    parts.push(k.download.installing);
  } else if (!done && !total) {
    parts.push(k.download.connecting);
  } else {
    parts.push(total ? k.download.progress(bytes(done), bytes(total)) : bytes(done));
    if (rate) {
      parts.push(k.download.speed(bytes(Math.round(rate))));
      if (total) {
        const left = Math.max(0, (total - done) / rate);
        parts.push(
          k.download.remaining(
            left < 60 ? k.download.seconds(Math.max(1, Math.round(left))) : k.download.minutes(Math.ceil(left / 60)),
          ),
        );
      }
    }
  }
  return (
    <div className="screen-download" role="status">
      <div className="screen-download__bar">
        <progress max={100} value={installing ? undefined : pct} aria-label={label} />
        {pct !== undefined && !installing && <span className="screen-download__pct">{`${pct}%`}</span>}
      </div>
      <p className="screen-download__line">{parts.join(" · ")}</p>
      <p className="screen-download__path" translate="no">
        {k.download.savedTo(homeRelative(path))}
      </p>
    </div>
  );
}

/** A model's download: the button and where it will be saved, how far it
 * is, or that it is ready. */
function ModelDownload({
  model,
  onChange,
  help,
}: {
  model: EmbeddingModel;
  onChange: () => void;
  /** What the model is for, where the page has not said it already. */
  help?: string;
}) {
  const client = useClient();
  const download = useAction();
  const start = async () => {
    if (await download.run(() => client.downloadModel({ id: model.id }))) onChange();
  };
  if (model.state === "ready") return <p className="screen-help">{k.modelReady}</p>;
  if (model.state === "downloading") {
    return <DownloadProgress done={model.done} total={model.size} path={model.path} label={model.name} />;
  }
  const failed = model.state === "failed";
  return (
    <div className="screen-knowledge-install">
      {(failed || help) && <p>{failed ? k.download.failed : help}</p>}
      <Button icon="download" disabled={download.busy} onClick={() => void start()}>
        {failed ? k.download.retry : `${k.modelDownload} (${bytes(model.size)})`}
      </Button>
      <p className="screen-download__path" translate="no">
        {k.download.savedTo(homeRelative(model.path))}
      </p>
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

/** What an index is built for: search by words, by words and meaning, or
 * analysis tables made of the collection's table files. */
type Purpose = "words" | "meaning" | "analysis";

/** Build a collection's index: what it is for (each purpose named and
 * explained), the extension and model it needs with their downloads, and
 * the first passages it will make. */
export function KnowledgeDialog({
  space,
  onClose,
  onPrepared,
  onAnalyze,
}: {
  space: string;
  onClose: () => void;
  onPrepared: () => void;
  /** Show the analysis tables made, once they are. */
  onAnalyze?: () => void;
}) {
  const client = useClient();
  const status = useLoad(() => client.knowledgeStatus({ space }), [space]);
  const preview = useLoad(() => client.knowledgePreview({ space }), [space]);
  const install = useAction();
  const prepare = useAction();
  const models = useModels();
  const [purpose, setPurpose] = useState<Purpose>("words");
  // The table files the collection holds, for Data Analysis.
  const tables = useLoad(() => client.fileAnalysis({ space }), [space]);
  const analyzable = (tables.data?.sources.length ?? 0) + (tables.data?.tables.length ?? 0) > 0;
  const build = useAction();
  const analyzing = purpose === "analysis";
  const [installed, setInstalled] = useState(false);
  const available = status.data?.available ?? false;
  const extension = status.data?.extension;
  const installing = extension?.state === "downloading" || extension?.state === "installing";
  const model = models.data?.[0];
  // Semantic search does not wait for the model: the index is built at
  // once and searched by words until the model has arrived.

  // While the extension downloads, look again every second.
  useEffect(() => {
    if (!installing) return;
    const timer = setTimeout(status.reload, 1000);
    return () => clearTimeout(timer);
  }, [status.data, installing, status.reload]);

  const doInstall = async () => {
    if (await install.run(() => client.installLance())) {
      setInstalled(true);
      status.reload();
    }
  };
  const doAnalyze = async () => {
    if (await build.run(() => client.buildAnalysis({ space }))) {
      onAnalyze?.();
      onClose();
    }
  };
  const doPrepare = async () => {
    const done = await prepare.run(async () => {
      await client.setKnowledgeModel({ space, model: purpose === "meaning" && model ? model.id : null });
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
          {analyzing ? (
            <Button variant="primary" disabled={!analyzable || build.busy} onClick={() => void doAnalyze()}>
              {build.busy ? strings.analysisView.making : strings.analysisView.make}
            </Button>
          ) : (
            <Button variant="primary" disabled={!available || prepare.busy} onClick={() => void doPrepare()}>
              {prepare.busy ? k.preparing : k.action}
            </Button>
          )}
        </>
      }
    >
      <p className="oik-dialog__text">{k.body}</p>
      <ChoiceList
        label={k.purposeLabel}
        name="knowledge-purpose"
        value={purpose}
        onChange={(value) => setPurpose(value === "meaning" || value === "analysis" ? value : "words")}
        choices={[
          { value: "words", title: k.purposes.words, meta: k.purposes.wordsHelp, leading: <Icon name="search" /> },
          ...(model
            ? [
                {
                  value: "meaning",
                  title: k.purposes.meaning,
                  meta: k.purposes.meaningHelp(model.name, bytes(model.size)),
                  leading: <Icon name="ai" />,
                  more:
                    model.state === "ready" ? undefined : model.state === "downloading" ? (
                      <DownloadProgress done={model.done} total={model.size} path={model.path} label={model.name} />
                    ) : (
                      <>
                        <p className="screen-help">{k.purposes.meaningLater}</p>
                        <p className="screen-download__path" translate="no">
                          {k.download.savedTo(homeRelative(model.path))}
                        </p>
                      </>
                    ),
                },
              ]
            : []),
          {
            value: "analysis",
            title: k.purposes.analysis,
            meta: k.purposes.analysisHelp,
            leading: <Icon name="table" />,
            disabled: tables.data !== undefined && !analyzable,
            more: !analyzable ? (
              <p className="screen-help">{k.purposes.analysisNone}</p>
            ) : (
              <ul className="screen-analysis__sources" aria-label={strings.analysisView.willMake}>
                {[...(tables.data?.sources ?? []), ...(tables.data?.tables ?? [])].map((source) => (
                  <li key={source.name}>
                    <span className="oik-mono" translate="no">
                      {source.name}
                    </span>
                    <span className="screen-help" translate="no" title={source.files.join("\n")}>
                      {source.files.length === 1 ? source.files[0] : strings.analysisView.files(source.files.length)}
                    </span>
                  </li>
                ))}
              </ul>
            ),
          },
        ]}
      />
      {!analyzing && installed && available && <p className="screen-help screen-knowledge-ready">{k.installed}</p>}
      {!analyzing && status.data && extension && !available && (
        <div className="screen-knowledge-install">
          <p>{extension.state === "failed" ? k.download.failed : k.install}</p>
          {installing ? (
            <DownloadProgress
              done={extension.done}
              total={extension.total}
              path={extension.path}
              label={k.installAction}
              installing={extension.state === "installing"}
            />
          ) : (
            <>
              <Button icon="download" disabled={install.busy} onClick={() => void doInstall()}>
                {extension.state === "failed" ? k.download.retry : k.installAction}
              </Button>
              <p className="screen-download__path" translate="no">
                {k.download.savedTo(homeRelative(extension.path))}
              </p>
            </>
          )}
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
      {build.error && <ErrorText>{build.error}</ErrorText>}
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
      ? model.state === "downloading"
        ? k.meaningDownloading
        : k.meaningNoModel(model.name)
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
            disabled={change.busy}
            onClick={() => void set(!status.model)}
          >
            {status.model ? k.turnOff : k.turnOn}
          </Button>
        }
      />
      {model.state !== "ready" && (
        <li className="oik-group__item screen-model-row">
          {status.model ? (
            // Chosen: how far its download is, or downloading it again.
            <ModelDownload model={model} onChange={models.reload} />
          ) : (
            // Not chosen: what enabling it downloads, and where to.
            <>
              <p className="screen-help">{k.purposes.meaningHelp(model.name, bytes(model.size))}</p>
              <p className="screen-download__path" translate="no">
                {k.download.savedTo(homeRelative(model.path))}
              </p>
            </>
          )}
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

/** The skills found in a prepared collection, for the owner to see what
 * assistants are offered and why one is not. */
function Skills({ space }: { space: string }) {
  const client = useClient();
  const skills = useLoad(() => client.knowledgeSkills({ space }), [space]);
  if (!skills.data?.length && !skills.error) return null;
  return (
    <Group title={k.skillsTitle} footnote={k.skillsFootnote}>
      {skills.error && <Row title={skills.error} muted />}
      {skills.data?.map((skill) => (
        <Row
          key={skill.dir || skill.name}
          leading={<Icon name="note" />}
          title={<span translate="no">{skill.name || skill.dir}</span>}
          meta={
            skill.problem
              ? `${k.skillOff}: ${k.skillProblems[skill.problem] ?? skill.problem}`
              : `${skill.description} · ${k.skillFiles(skill.files.length)}`
          }
          trailing={
            <span className="screen-skill-dir" translate="no">
              {skill.dir ? `${skill.dir}/` : "/"}
            </span>
          }
        />
      ))}
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
              <ModelDownload model={model} onChange={models.reload} help={k.modelPurpose} />
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
export function KnowledgeView({ space, onAnalyze }: { space: string; onAnalyze?: () => void }) {
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
        {preparing && (
          <KnowledgeDialog
            space={space}
            onClose={() => setPreparing(false)}
            onPrepared={status.reload}
            onAnalyze={onAnalyze}
          />
        )}
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
      <Skills space={space} />
      {retry.error && <ErrorText>{retry.error}</ErrorText>}
    </div>
  );
}
