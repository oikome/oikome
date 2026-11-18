// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Knowledge an AI can read (docs/knowledge-schema.md, PR 28 K1): Prepare,
// where each original is, its passages, and a retrieval test that searches
// the way an assistant does.

import { Fragment, useEffect, useState } from "react";
import type { FormEvent, ReactNode } from "react";
import { Button, Card, ChoiceList, Dialog, EmptyState, ErrorText, Group, Icon, Row, Status, TextField } from "@oikome/ui";
import type { StatusKind } from "@oikome/ui";
import type { KnowledgeChunk, KnowledgeFile, KnowledgeStage, RetrievalResults } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { strings } from "../strings";

const k = strings.knowledge;

const KINDS: Record<KnowledgeStage, StatusKind> = {
  waiting: "idle",
  reading: "run",
  splitting: "run",
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
  const [format, setFormat] = useState("lance");
  const [installed, setInstalled] = useState(false);
  const available = status.data?.available ?? false;

  const doInstall = async () => {
    if (await install.run(() => client.installLance())) {
      setInstalled(true);
      status.reload();
    }
  };
  const doPrepare = async () => {
    if (await prepare.run(() => client.prepareKnowledge({ space }))) {
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
            disabled={!available || format !== "lance" || prepare.busy}
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

function RetrievalTest({ space }: { space: string }) {
  const client = useClient();
  const search = useAction();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<RetrievalResults>();
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!query.trim()) return;
    const found = await search.run(() => client.retrievalTest({ space, query: query.trim(), limit: 10 }));
    if (found) setResults(found);
  };
  return (
    <Group title={k.test} footnote={k.testHelp}>
      <li className="oik-group__item">
        <form className="screen-retrieval" onSubmit={(e) => void submit(e)}>
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
                        {hit.score != null && <span>{k.score(hit.score.toFixed(2))}</span>}
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

/** A prepared collection's knowledge: progress, each original and its
 * passages, and the retrieval test. */
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
      <RetrievalTest space={space} />
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
