// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// A source, opened: what it brings into which collection, how it is doing,
// its recent syncs, and its settings (how often it is read, pausing it, and
// removing it), as the prototype's source page.

import { useState } from "react";
import { Banner, Button, Card, ConfirmDialog, EmptyState, ErrorText, Segmented, Status, Switch, useToast } from "@oikome/ui";
import type { SourceRun, SourceSummary } from "@oikome/client";
import { forgetPreviews } from "../content";
import { useAction, useClient, useLoad } from "../data";
import { dayAndTime, homeRelative, sameDayTime } from "../format";
import { ScreenHeader } from "../parts";
import { Link, navigate } from "../router";
import { strings } from "../strings";
import { Holds, lastText, nextText, placeName, statusOf } from "./Sources";

const s = strings.sources;

/** How often a source can be read, in seconds; `ask` only when asked. */
const EVERY: { value: string; label: string }[] = [
  { value: "60", label: s.every.minute },
  { value: "300", label: s.every.five },
  { value: "1800", label: s.every.thirty },
  { value: "3600", label: s.every.hourly },
  { value: "86400", label: s.every.daily },
  { value: "ask", label: s.every.ask },
];

/** What a sync did, in words. */
function runText(run: SourceRun): string {
  const since = run.count > 1 ? ` ${s.runSince(sameDayTime(run.firstMs))}` : "";
  if (run.failure) return `${run.failure}${since}`;
  const parts = [
    run.upserted > 0 ? s.runChanged(run.upserted) : null,
    run.deleted > 0 ? s.runRemoved(run.deleted) : null,
  ].filter(Boolean);
  return parts.length > 0 ? parts.join(", ") : `${s.runNothing}${since}`;
}

export function SourceDetail({ name }: { name: string }) {
  const client = useClient();
  const toast = useToast();
  const sources = useLoad(() => client.listSources(), []);
  const spaces = useLoad(() => client.listSpaces(), []);
  const runs = useLoad(() => client.listSourceRuns({ source: name }), [name]);
  const sync = useAction();
  const change = useAction();
  const remove = useAction();
  const [removing, setRemoving] = useState(false);

  if (sources.error) return <ErrorText>{sources.error}</ErrorText>;
  if (!sources.data) return <div className="screen-page-skeleton oik-skel" aria-hidden="true" />;
  const source = sources.data.find((x) => x.name === name);
  if (!source) {
    return (
      <Card>
        <EmptyState title={s.goneTitle} body={s.goneBody} action={<Link to="/sources">{strings.nav.sources}</Link>} />
      </Card>
    );
  }
  const space = spaces.data?.find((x) => x.name === name);
  const status = statusOf(source);
  const title = placeName(source.place);
  const into = (
    <Link to={`/collections/${encodeURIComponent(source.name)}`} className="oik-link screen-link-mono">
      {source.name}
    </Link>
  );
  const reload = () => {
    sources.reload();
    runs.reload();
  };
  const set = async (next: () => Promise<SourceSummary>, said?: (done: SourceSummary) => string) => {
    const done = await change.run(next);
    if (done) {
      if (said) toast(said(done));
      reload();
    }
  };

  return (
    <>
      <ScreenHeader label={s.label} title={title} lede={<>{s.brings(s.what[source.kind] ?? s.what.csv)} {into}.</>}>
        <Button
          variant="primary"
          icon="sync"
          disabled={sync.busy}
          spinning={sync.busy}
          onClick={async () => {
            const done = await sync.run(() => client.pullSource({ source: source.name }));
            forgetPreviews(source.name);
            reload();
            if (done) toast(s.synced(title));
          }}
        >
          {s.syncNow}
        </Button>
      </ScreenHeader>
      {sync.error && <ErrorText>{sync.error}</ErrorText>}

      {status.kind === "bad" && source.failure && <Banner title={source.failure} body={s.badBody(source.name)} />}

      <dl className="oik-card screen-facts">
        <div>
          <dt className="oik-label">{s.colStatus}</dt>
          <dd>
            <Status kind={status.kind}>{status.label}</Status>
          </dd>
        </div>
        <div>
          <dt className="oik-label">{s.colWhen}</dt>
          <dd>
            {lastText(source)} · {nextText(source)}
          </dd>
        </div>
        <div>
          <dt className="oik-label">{s.colHolds}</dt>
          <dd>
            <Holds space={space} />
          </dd>
        </div>
        <div>
          <dt className="oik-label">{s.reads}</dt>
          <dd translate="no" title={source.place}>
            {source.kind === "photos" || source.kind === "notes" ? s.readsFolder(homeRelative(source.place)) : s.readsFile(homeRelative(source.place))}
          </dd>
        </div>
      </dl>

      <div className="screen-dash screen-dash--split screen-source-page">
        <section className="oik-card screen-panel" aria-label={s.recent}>
          <header className="screen-panel__head">
            <h2 className="oik-label">{s.recent}</h2>
            <span className="screen-panel__aside">{s.kept30}</span>
          </header>
          {runs.error ? (
            <p className="screen-panel__empty">{runs.error}</p>
          ) : !runs.data ? (
            <span className="oik-skel screen-panel__skeleton" aria-hidden="true" />
          ) : runs.data.length === 0 ? (
            <p className="screen-panel__empty">{s.noRuns}</p>
          ) : (
            <ol className="screen-runs">
              {runs.data.map((run) => (
                <li key={`${run.firstMs}-${run.atMs}`} className="screen-runs__row">
                  <span className="screen-runs__when">{dayAndTime(run.atMs)}</span>
                  <span className="screen-runs__what">{runText(run)}</span>
                  <Status kind={run.failure ? "bad" : "ok"}>{run.failure ? s.failed : s.done}</Status>
                </li>
              ))}
            </ol>
          )}
        </section>

        <section className="oik-card screen-panel" aria-label={strings.nav.settings}>
          <header className="screen-panel__head">
            <h2 className="oik-label">{strings.nav.settings}</h2>
          </header>
          <div className="screen-source-settings">
            <div className="screen-source-settings__field">
              <span className="oik-label">{s.checkNew}</span>
              <Segmented
                label={s.checkNew}
                value={source.everySecs == null ? "ask" : String(source.everySecs)}
                onChange={(value) =>
                  void set(() =>
                    client.setSourceEvery({ source: source.name, everySecs: value === "ask" ? null : Number(value) }),
                  )
                }
                options={EVERY}
              />
            </div>
            <div className="screen-source-settings__row">
              <span>{s.pauseThis}</span>
              <Switch
                label={s.pauseThis}
                checked={source.paused}
                disabled={change.busy}
                onChange={(paused) =>
                  void set(
                    () => client.pauseSource({ source: source.name, paused }),
                    (done) => (done.paused ? s.pausedToast(title) : s.resumed(title)),
                  )
                }
              />
            </div>
            <div className="screen-source-settings__row">
              <span>{s.goesInto}</span>
              {into}
            </div>
            {change.error && <ErrorText>{change.error}</ErrorText>}
          </div>
          <footer className="screen-panel__foot">
            <Button variant="quiet" onClick={() => setRemoving(true)}>
              {s.removeShort}
            </Button>
          </footer>
        </section>
      </div>

      {removing && (
        <ConfirmDialog
          title={s.removeTitle(title)}
          body={s.removeBody(source.name, title)}
          confirmLabel={s.removeYes}
          cancelLabel={strings.common.cancel}
          busy={remove.busy}
          error={remove.error}
          onConfirm={async () => {
            const done = await remove.run(() => client.removeSource({ source: source.name }));
            if (done) {
              toast(s.removed(title));
              navigate("/sources", { replace: true });
            }
          }}
          onCancel={() => {
            setRemoving(false);
            remove.clear();
          }}
        />
      )}
    </>
  );
}
