// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// MCP Access: the clients the owner connected over MCP, what each can
// read (switched on and off in place), and what it searched and read
// lately, from Activity. Connecting one takes three steps: the app, what it
// can read, and the command (or address and header) to paste, with its key
// shown that once; the page waits until the assistant first reaches the
// data center.

import { useEffect, useState } from "react";
import {
  Button,
  Checkbox,
  ChoiceList,
  ConfirmDialog,
  CopyField,
  Dialog,
  ErrorText,
  Group,
  Icon,
  Row,
  Select,
  Status,
  StepDots,
  Switch,
  Tile,
  useToast,
} from "@oikome/ui";
import { OikomeError } from "@oikome/client";
import type { Assistant, AssistantConnected, McpClientCapability, SpaceSummary } from "@oikome/client";
import { errorMessage, useAction, useClient, useLoad } from "../data";
import { inLanguage } from "../plugins";
import { ago, calendarDate } from "../format";
import { ScreenHeader } from "../parts";
import { Link, navigate } from "../router";
import { strings } from "../strings";
import { sentence } from "./Activity";

const s = strings.assistants;
/** Used within this long: shown as connected; idle after. */
const RECENT_MS = 15 * 60 * 1000;
/** How often the connect dialog looks whether the assistant reached us. */
const WAIT_EVERY_MS = 2000;

interface ClientChoice {
  key: string;
  name: string;
  plugin: string;
  client: string;
  capability: McpClientCapability;
}

function metaOf(a: Assistant): string {
  if (a.endedAtMs != null) return s.ended(calendarDate(a.endedAtMs));
  return [s.reads(a.collections), a.lastUsedMs != null ? s.lastUsed(ago(a.lastUsedMs)) : s.notYet].join(" · ");
}

function StatusOf({ a }: { a: Assistant }) {
  if (a.endedAtMs != null) return <Status kind="off">{s.statusEnded}</Status>;
  const recent = a.lastUsedMs != null && Date.now() - a.lastUsedMs < RECENT_MS;
  return <Status kind={recent ? "ok" : "idle"}>{recent ? s.statusConnected : s.statusIdle}</Status>;
}

export function Assistants({ startConnecting = false }: { startConnecting?: boolean }) {
  const client = useClient();
  const toast = useToast();
  const assistants = useLoad(() => client.listAssistants(), []);
  const spaces = useLoad(() => client.listSpaces(), []);
  const plugins = useLoad(async () => (await client.listPlugins()).map(inLanguage), []);
  const [connecting, setConnecting] = useState(startConnecting);
  const [editing, setEditing] = useState<Assistant>();
  const [removing, setRemoving] = useState<Assistant>();
  const [picked, setPicked] = useState<string>();
  const grant = useAction();
  const remove = useAction();

  const list = assistants.data ?? [];
  const active = list.filter((a) => a.endedAtMs == null);
  const current = active.find((a) => a.id === picked) ?? active[0];
  const mine: SpaceSummary[] = (spaces.data ?? []).filter((space) => space.kind !== "remote");
  const clients: ClientChoice[] = (plugins.data ?? []).filter((plugin) => plugin.installed).flatMap((plugin) =>
    plugin.capabilities.mcpClients.map((capability) => ({
      key: `${plugin.id}:${capability.id}`,
      name: capability.name,
      plugin: plugin.id,
      client: capability.id,
      capability,
    })),
  );

  const give = async (a: Assistant, collections: string[]) => {
    const saved = await grant.run(() => client.setAssistantCollections({ id: a.id, collections }));
    if (saved) assistants.reload();
  };

  return (
    <>
      <ScreenHeader title={s.title} lede={s.lede}>
        <Button
          variant="primary"
          icon="plus"
          disabled={plugins.data !== undefined && clients.length === 0}
          onClick={() => setConnecting(true)}
        >
          {s.connect}
        </Button>
      </ScreenHeader>
      {plugins.data && clients.length === 0 && <p className="screen-help">{s.noClients}</p>}
      {(assistants.error ?? spaces.error ?? grant.error) && (
        <ErrorText>{assistants.error ?? spaces.error ?? grant.error}</ErrorText>
      )}

      <div className="screen-dash screen-dash--split screen-dash--rows">
        <Group title={s.connected} action={<span className="screen-group-count">{s.overMcp}</span>}>
          {assistants.data && list.length === 0 && <Row title={s.none} muted />}
          {assistants.data &&
            list.length === 0 &&
            clients.map((choice) => (
              <Row
                key={choice.key}
                leading={<Tile icon="ai" tone="stone" />}
                title={choice.name}
                trailing={
                  <Button variant="quiet" aria-label={`${s.connect}: ${choice.name}`} onClick={() => setConnecting(true)}>
                    {s.connect}
                  </Button>
                }
              />
            ))}
          {list.map((a) => (
            <Row
              key={a.id}
              leading={<Tile icon="ai" tone={a.endedAtMs == null ? "lavender" : "stone"} />}
              title={a.name}
              meta={<span translate="no">{metaOf(a)}</span>}
              trailing={
                <span className="screen-assistant-actions">
                  <StatusOf a={a} />
                  {a.endedAtMs == null ? (
                    <Button variant="quiet" aria-label={s.editFor(a.name)} onClick={() => setEditing(a)}>
                      {s.edit}
                    </Button>
                  ) : (
                    <Button variant="quiet" aria-label={s.removeFor(a.name)} onClick={() => setRemoving(a)}>
                      {s.remove}
                    </Button>
                  )}
                </span>
              }
            />
          ))}
        </Group>

        <Group
          title={current ? s.canRead(current.name) : s.canRead(s.title)}
          action={
            active.length > 1 && current ? (
              <Select
                inline
                label={s.connected}
                value={current.id}
                onChange={setPicked}
                options={active.map((a) => ({ value: a.id, label: a.name }))}
              />
            ) : undefined
          }
          footnote={current && mine.length > 0 ? s.canReadHint : undefined}
        >
          {!current && <Row title={s.nothingToChange} muted />}
          {current && spaces.data && mine.length === 0 && <Row title={s.noCollections} muted />}
          {current &&
            mine.map((space) => {
              const on = current.collections.includes(space.name);
              return (
                <Row
                  key={space.name}
                  title={
                    <span className="oik-mono" translate="no">
                      {space.name}
                    </span>
                  }
                  trailing={
                    <Switch
                      label={`${s.canRead(current.name)}: ${space.name}`}
                      checked={on}
                      disabled={grant.busy}
                      onChange={(next) =>
                        void give(
                          current,
                          next
                            ? [...current.collections, space.name]
                            : current.collections.filter((c) => c !== space.name),
                        )
                      }
                    />
                  }
                />
              );
            })}
        </Group>
      </div>

      {current && <Lately key={current.id} assistant={current} />}

      {connecting && (
        <ConnectDialog
          mine={mine}
          clients={clients}
          onClose={() => {
            setConnecting(false);
            assistants.reload();
            if (startConnecting) navigate("/assistants", { replace: true });
          }}
          onConnected={(a) => {
            setPicked(a.id);
            assistants.reload();
          }}
        />
      )}
      {editing && (
        <EditDialog
          assistant={editing}
          mine={mine}
          onClose={() => setEditing(undefined)}
          onChanged={(message) => {
            setEditing(undefined);
            toast(message);
            assistants.reload();
          }}
        />
      )}
      {removing && (
        <ConfirmDialog
          title={s.removeTitle(removing.name)}
          body={s.removeBody}
          confirmLabel={s.remove}
          cancelLabel={strings.common.cancel}
          busy={remove.busy}
          error={remove.error}
          onConfirm={async () => {
            const left = await remove.run(() => client.removeAssistant({ id: removing.id }));
            if (left) {
              toast(s.removedToast(removing.name));
              setRemoving(undefined);
              assistants.reload();
            }
          }}
          onCancel={() => {
            setRemoving(undefined);
            remove.clear();
          }}
        />
      )}
    </>
  );
}

/** What an assistant searched and read lately: its own lines of Activity. */
function Lately({ assistant }: { assistant: Assistant }) {
  const client = useClient();
  const lines = useLoad(() => client.listAccess({ did: assistant.did, limit: 8 }), [assistant.did, assistant.lastUsedMs]);
  return (
    <section className="screen-section screen-assistant-lately">
      <Group
        title={s.lately(assistant.name)}
        action={<Link to="/activity">{s.openActivity}</Link>}
        footnote={s.latelyNote}
      >
        {lines.error && <Row title={lines.error} muted />}
        {lines.data && lines.data.length === 0 && <Row title={s.latelyNone(assistant.name)} muted />}
        {(lines.data ?? []).map((line) => (
          <Row
            key={line.id}
            title={sentence(line)}
            meta={ago(line.atMs)}
            trailing={
              line.outcome === "allowed" ? undefined : (
                <Status kind="bad">{strings.activity.refused}</Status>
              )
            }
          />
        ))}
      </Group>
    </section>
  );
}

/** The collections of one's own, each with a box. */
function Choose({
  mine,
  chosen,
  onChange,
  label,
}: {
  mine: SpaceSummary[];
  chosen: Set<string>;
  onChange: (next: Set<string>) => void;
  label: string;
}) {
  return (
    <fieldset className="screen-plugin-reads">
      <legend className="oik-field__label">{label}</legend>
      {mine.length === 0 ? (
        <p className="screen-help">{s.noCollections}</p>
      ) : (
        <ul className="screen-lend-people">
          {mine.map((space) => (
            <li key={space.name}>
              <Checkbox
                label={<span className="screen-cell-mono">{space.name}</span>}
                checked={chosen.has(space.name)}
                onChange={(on) => {
                  const next = new Set(chosen);
                  if (on) next.add(space.name);
                  else next.delete(space.name);
                  onChange(next);
                }}
              />
            </li>
          ))}
        </ul>
      )}
    </fieldset>
  );
}

function ConnectDialog({
  mine,
  clients,
  onClose,
  onConnected,
}: {
  mine: SpaceSummary[];
  clients: ClientChoice[];
  onClose: () => void;
  onConnected: (a: Assistant) => void;
}) {
  const client = useClient();
  const toast = useToast();
  const [step, setStep] = useState(1);
  const [picked, setPicked] = useState("");
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [done, setDone] = useState<AssistantConnected>();
  const [reached, setReached] = useState(false);
  const [configuring, setConfiguring] = useState(false);
  const [configured, setConfigured] = useState(false);
  const [manual, setManual] = useState(false);
  const [replacing, setReplacing] = useState(false);
  const [configError, setConfigError] = useState<string>();
  const connect = useAction();
  const selected = clients.find((choice) => choice.key === picked) ?? clients[0];
  const capability = selected?.capability;
  const shown = selected?.name ?? "";

  // Until the assistant first reaches the data center, look now and then.
  useEffect(() => {
    if (!done || reached) return;
    const timer = window.setInterval(() => {
      client
        .listAssistants()
        .then((all) => {
          const a = all.find((x) => x.id === done.assistant.id);
          if (a?.lastUsedMs != null) {
            setReached(true);
            onConnected(a);
          }
        })
        .catch(() => undefined);
    }, WAIT_EVERY_MS);
    return () => window.clearInterval(timer);
  }, [client, done, reached, onConnected]);

  const endpoint = `${window.location.origin}/mcp`;
  const configuration = !done
    ? ""
    : capability?.command
      ? capability.command.replaceAll("{url}", endpoint).replaceAll("{token}", done.token)
      : capability?.config?.format === "json"
        ? JSON.stringify(
            {
              [capability.config.root]: {
                oikome: { url: endpoint, [capability.config.headers]: { Authorization: `Bearer ${done.token}` } },
              },
            },
            null,
            2,
          )
        : capability?.config?.format === "toml"
          ? `[${capability.config.root}.oikome]\nurl = ${JSON.stringify(endpoint)}\n${capability.config.headers} = { Authorization = ${JSON.stringify(`Bearer ${done.token}`)} }`
          : "";
  const instruction = capability?.command
    ? s.pasteCommand(shown)
    : capability?.config
      ? s.pasteConfig(shown)
      : s.pasteOther(shown);

  const configure = async (replace: boolean) => {
    if (!done) return;
    setConfiguring(true);
    setConfigError(undefined);
    try {
      await client.installAssistant({ id: done.assistant.id, token: done.token, url: endpoint, replace });
      setConfigured(true);
      setReplacing(false);
    } catch (reason) {
      if (reason instanceof OikomeError && reason.code === "conflict") setReplacing(true);
      else setConfigError(errorMessage(reason));
    } finally {
      setConfiguring(false);
    }
  };

  let body;
  let footer;
  if (step === 1) {
    body = (
      <ChoiceList
        label={s.stepApp}
        name="assistant-app"
        value={selected?.key ?? ""}
        onChange={setPicked}
        choices={clients.map((choice) => ({
          value: choice.key,
          title: choice.name,
          leading: <Icon name="terminal" />,
        }))}
      />
    );
    footer = (
      <>
        <Button onClick={onClose}>{strings.common.cancel}</Button>
        <Button variant="primary" disabled={!selected} onClick={() => setStep(2)}>
          {s.next}
        </Button>
      </>
    );
  } else if (step === 2) {
    body = (
      <>
        <p className="oik-dialog__text">{s.nothingUntil}</p>
        <Choose mine={mine} chosen={chosen} onChange={setChosen} label={s.stepReads} />
        {connect.error && <ErrorText>{connect.error}</ErrorText>}
      </>
    );
    footer = (
      <>
        <Button onClick={() => setStep(1)}>{s.back}</Button>
        <Button
          variant="primary"
          disabled={connect.busy}
          onClick={async () => {
            const connected = await connect.run(() =>
              client.connectAssistant({
                name: shown,
                app: "other",
                ...(selected ? { plugin: selected.plugin, client: selected.client } : {}),
                collections: [...chosen],
              }),
            );
            if (connected) {
              setDone(connected);
              setStep(3);
              toast(s.connectedToast(connected.assistant.name));
              onConnected(connected.assistant);
            }
          }}
        >
          {s.nextConnect}
        </Button>
      </>
    );
  } else {
    const oneClick = capability?.config !== undefined;
    body = (
      <>
        <p className="oik-dialog__text">{oneClick ? s.oneClick(shown) : instruction}</p>
        {oneClick && (
          <div className="screen-assistant-install">
            {configured ? (
              <Status kind="ok">{s.configured(shown)}</Status>
            ) : (
              <Button variant="primary" disabled={configuring} onClick={() => void configure(false)}>
                {s.connectNow(shown)}
              </Button>
            )}
            <Button variant="quiet" onClick={() => setManual((shown) => !shown)}>
              {manual ? s.manualHide : s.manualShow}
            </Button>
          </div>
        )}
        {configError && !replacing && <ErrorText>{configError}</ErrorText>}
        {(!oneClick || manual) && (
          <div className="screen-assistant-config">
            <CopyField value={configuration} copyLabel={strings.common.copy} copiedLabel={strings.common.copied} />
          </div>
        )}
        {(!oneClick || manual) && <p className="screen-help">{s.shownOnce}</p>}
        {(!oneClick || configured || manual) && (
          <p className="screen-assistant-wait" role="status" aria-live="polite">
            {reached ? <Status kind="ok">{s.isConnected(shown)}</Status> : <Status kind="run">{s.waiting(shown)}</Status>}
          </p>
        )}
        <p className="screen-help">
          {s.willRead([...chosen])} {s.takeBack}
        </p>
      </>
    );
    footer = reached ? (
      <Button variant="primary" onClick={onClose}>
        {strings.common.done}
      </Button>
    ) : (
      <Button onClick={onClose}>{strings.common.close}</Button>
    );
  }

  return (
    <>
      <Dialog title={s.connectTitle} closeLabel={strings.common.close} onClose={onClose} footer={footer}>
        <StepDots step={step} total={3} label={s.steps} />
        {body}
      </Dialog>
      {replacing && done && (
        <ConfirmDialog
          title={s.replaceTitle(shown)}
          body={s.replaceBody(shown)}
          confirmLabel={s.replaceConfirm}
          cancelLabel={strings.common.cancel}
          busy={configuring}
          error={configError}
          onConfirm={() => void configure(true)}
          onCancel={() => {
            setReplacing(false);
            setConfigError(undefined);
          }}
        />
      )}
    </>
  );
}

function EditDialog({
  assistant,
  mine,
  onClose,
  onChanged,
}: {
  assistant: Assistant;
  mine: SpaceSummary[];
  onClose: () => void;
  onChanged: (message: string) => void;
}) {
  const client = useClient();
  const save = useAction();
  const end = useAction();
  const [chosen, setChosen] = useState(() => new Set(assistant.collections));
  const [ending, setEnding] = useState(false);
  if (ending) {
    return (
      <ConfirmDialog
        title={s.endTitle(assistant.name)}
        body={s.endBody}
        confirmLabel={s.endYes}
        cancelLabel={strings.common.cancel}
        busy={end.busy}
        error={end.error}
        onConfirm={async () => {
          const ended = await end.run(() => client.endAssistantAccess({ id: assistant.id }));
          if (ended) onChanged(s.endedToast(assistant.name));
        }}
        onCancel={() => {
          setEnding(false);
          end.clear();
        }}
      />
    );
  }
  return (
    <Dialog
      title={assistant.name}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button variant="destructive" className="screen-dialog-start" onClick={() => setEnding(true)}>
            {s.endAccess}
          </Button>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button
            variant="primary"
            disabled={save.busy}
            onClick={async () => {
              const saved = await save.run(() =>
                client.setAssistantCollections({ id: assistant.id, collections: [...chosen] }),
              );
              if (saved) onChanged(s.savedToast(assistant.name));
            }}
          >
            {strings.common.save}
          </Button>
        </>
      }
    >
      <Choose mine={mine} chosen={chosen} onChange={setChosen} label={s.editCanRead} />
      <p className="screen-help">{s.editHint}</p>
      {save.error && <ErrorText>{save.error}</ErrorText>}
    </Dialog>
  );
}
