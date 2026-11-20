// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Settings › Backup (PR 30). Off until the owner turns it on with where
// backups go (one to four places) and when; nothing is backed up until that
// time or Back Up Now. Then each place with its last backup and check, the
// schedule, how many are kept, a job's progress, the backups kept at a
// place, and Restore from Backup, which puts a backup of this very data
// folder in place when Oikome is next started.

import { useEffect, useState } from "react";
import {
  Banner,
  Button,
  ChoiceList,
  Dialog,
  ErrorText,
  Group,
  GroupPanel,
  IconButton,
  Row,
  Segmented,
  Select,
  Status,
  Switch,
  TextArea,
  Tile,
  useToast,
} from "@oikome/ui";
import type { IconName, TileTone } from "@oikome/ui";
import type {
  BackupEvery,
  BackupInfo,
  BackupItem,
  BackupPlace,
  BackupProgress,
  BackupSchedule,
  RestoredBackup,
  StorageSummary,
} from "@oikome/client";
import { asSentence, useAction, useClient, useLoad } from "../data";
import { bytes, dateTime, dayAndTime, homeRelative, listNames } from "../format";
import { sentence } from "../i18n/en";
import { locale } from "../i18n/languages";
import { TransferProgress } from "../progress";
import { FolderBrowser, STORAGE_LOOK } from "../storage";
import { strings } from "../strings";

const d = strings.dataBackup;

/** The folder backups are kept in, inside the one chosen (as the node names it). */
const REPOSITORY_FOLDER = "oikome-backup";
/** The most places backups go to at once (as the node allows). */
const MAX_PLACES = 4;
/** Storages a backup can go to: a folder, a cloud drive's folder, a NAS, RustFS. */
const BACKUP_KINDS: StorageSummary["type"][] = ["folder", "drive", "webdav", "s3"];
const THIS_COMPUTER: [IconName, TileTone] = ["computer", "stone"];
/** Restore from Backup's choice of a folder that is not a backup place. */
const ANOTHER_FOLDER = "folder";

/** A place as the owner reads it: the node names this computer in English. */
function placeLabel(place: BackupPlace): string {
  return place.place === null ? d.thisComputer : place.label;
}

/** Where at the place: a path on this computer, or a folder within a storage. */
function placePath(place: { place?: string | null; path: string }): string {
  return place.place ? `/${place.path.replace(/^\/+/, "")}` : homeRelative(place.path);
}

function lookOf(place: string | null | undefined, storages: StorageSummary[] | undefined): [IconName, TileTone] {
  if (!place) return THIS_COMPUTER;
  const storage = storages?.find((x) => x.name === place);
  return storage ? STORAGE_LOOK[storage.type] : ["folder", "oat"];
}

/** A time of day as the language writes it: "3:00 AM", "03:00". */
function clock(hour: number, minute: number): string {
  return new Intl.DateTimeFormat(locale(), { hour: "numeric", minute: "2-digit" }).format(
    new Date(2024, 0, 1, hour, minute),
  );
}

/** A day of the week by the node's number, 0 Monday … 6 Sunday ("Monday", "lundi"). */
function weekdayName(day: number): string {
  // 1 January 2024 was a Monday.
  return new Intl.DateTimeFormat(locale(), { weekday: "long" }).format(new Date(2024, 0, 1 + day));
}

function capitalized(text: string): string {
  return text.charAt(0).toLocaleUpperCase(locale()) + text.slice(1);
}

function scheduleText(schedule: BackupSchedule): string {
  const at = clock(schedule.hour, schedule.minute);
  return schedule.every === "weekly" ? d.everyWeek(weekdayName(schedule.weekday ?? 0), at) : d.everyDay(at);
}

const pad = (n: number) => String(n).padStart(2, "0");
const hhmm = (hour: number, minute: number) => `${pad(hour)}:${pad(minute)}`;

/** The step a job is at, with its place as the owner reads it. */
function stepText(running: BackupProgress, places: BackupPlace[]): string {
  if (running.step === "copying") return d.copying;
  if (running.place === null || running.index === 0) return d.preparing;
  // The node names a place on this computer in English; any place by that
  // label which is on this computer is named in the language in use.
  const local = places.some((p) => p.place === null && p.label === running.place);
  const where = local ? d.thisComputer : running.place;
  return running.step === "sending"
    ? d.sending(where, running.index, running.of)
    : d.checking(where, running.index, running.of);
}

/** Whether `child` is `parent` or inside it, as paths are written. */
function inside(child: string, parent: string): boolean {
  const trim = (path: string) => path.replace(/[\\/]+$/, "");
  const [a, b] = [trim(child), trim(parent)];
  return a === b || a.startsWith(`${b}/`) || a.startsWith(`${b}\\`);
}

/** A folder on this computer's own disk, rather than one plugged in. */
function onOwnDisk(path: string): boolean {
  return path.startsWith("/") && !/^\/(Volumes|media|mnt|run\/media)\//.test(path);
}

/** Settings › Backup, looked at again every 1.5 s while a job runs. */
function useBackupInfo() {
  const client = useClient();
  const loaded = useLoad(() => client.backupInfo(), [], asSentence);
  const [info, setInfo] = useState<BackupInfo>();
  useEffect(() => {
    if (loaded.data) setInfo(loaded.data);
  }, [loaded.data]);
  const running = info?.running != null;
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(loaded.reload, 1500);
    return () => clearInterval(timer);
  }, [running, loaded.reload]);
  return { info, error: loaded.error, setInfo };
}

/** Lines up a place's last backup and check under its name. */
const Indent = () => <span className="screen-backup-indent" aria-hidden="true" />;

/** A place, with how its last backup and its last check ended. */
function PlaceRows({
  place,
  look,
  onList,
}: {
  place: BackupPlace;
  look: [IconName, TileTone];
  onList: () => void;
}) {
  const backup = place.lastBackup;
  const check = place.lastCheck;
  const made = backup?.backup;
  return (
    <>
      <Row
        leading={<Tile icon={look[0]} tone={look[1]} />}
        title={placeLabel(place)}
        meta={
          <span className="screen-ellipsis" translate="no" title={place.path}>
            {placePath(place)}
          </span>
        }
        trailing={
          <Button variant="quiet" onClick={onList}>
            {d.showBackups}
          </Button>
        }
      />
      <Row
        leading={<Indent />}
        overline={d.lastBackup}
        title={backup ? dayAndTime(backup.atMs) : d.notBackedUp}
        muted={!backup}
        meta={
          backup?.error
            ? sentence(backup.error)
            : made
              ? d.sizeAdded(bytes(made.bytes), bytes(made.added))
              : undefined
        }
        trailing={
          backup && <Status kind={backup.error ? "bad" : "ok"}>{backup.error ? d.failed : d.done}</Status>
        }
      />
      <Row
        leading={<Indent />}
        overline={d.lastCheck}
        title={check ? dayAndTime(check.atMs) : d.notChecked}
        muted={!check}
        meta={check?.error ? sentence(check.error) : undefined}
        trailing={check && <Status kind={check.error ? "bad" : "ok"}>{check.error ? d.problem : d.ok}</Status>}
      />
    </>
  );
}

/** Settings › Backup. `dataDir` is this data center's data folder, which backups cannot go into. */
export function BackupSettings({ dataDir }: { dataDir: string }) {
  const client = useClient();
  const toast = useToast();
  const { info, error, setInfo } = useBackupInfo();
  const storages = useLoad(() => client.listStorages(), []);
  const act = useAction();
  const [editing, setEditing] = useState(false);
  const [listing, setListing] = useState<number>();
  const [restoring, setRestoring] = useState(false);
  const [restored, setRestored] = useState<RestoredBackup>();

  if (!info) {
    return error ? (
      <Group title={d.title}>
        <Row title={error} muted />
      </Group>
    ) : null;
  }
  const running = info.running;
  const busy = act.busy || running !== null;
  const run = async (action: () => Promise<BackupInfo>, said?: string) => {
    const updated = await act.run(action, asSentence);
    if (updated) {
      setInfo(updated);
      if (said) toast(said);
    }
  };
  const kept = d.kept(info.keepDaily, info.keepWeekly, info.keepMonthly);
  const shown = listing === undefined ? undefined : info.places[listing];

  return (
    <Group
      title={d.title}
      action={
        <Switch
          label={d.automatic}
          checked={info.enabled}
          disabled={busy}
          onChange={(on) => (on ? setEditing(true) : void run(() => client.turnOffBackup(), d.turnedOff))}
        />
      }
      footnote={info.enabled ? kept : undefined}
    >
      {!info.enabled && (
        <Row
          leading={<Tile icon="archive" tone="sage" />}
          alignStart
          title={
            <span className="screen-row-stack">
              <span className="screen-row-prose">{d.what}</span>
              <span className="screen-row-note">{info.places.length ? d.offKept : d.off}</span>
            </span>
          }
          trailing={
            <Button disabled={busy} onClick={() => setEditing(true)}>
              {d.turnOn}
            </Button>
          }
        />
      )}
      {info.enabled &&
        info.places.map((place, i) => (
          <PlaceRows
            key={`${place.place ?? ""}:${place.path}`}
            place={place}
            look={lookOf(place.place, storages.data)}
            onList={() => setListing(i)}
          />
        ))}
      {info.enabled && info.schedule && (
        <Row
          overline={d.schedule}
          title={scheduleText(info.schedule)}
          meta={info.nextAtMs != null ? d.next(dayAndTime(info.nextAtMs)) : undefined}
        />
      )}
      {running && (
        <GroupPanel>
          <TransferProgress
            // A new step starts its count, and its speed, again.
            key={`${running.work}:${running.step}:${running.index}`}
            done={running.done}
            total={running.total || null}
            label={stepText(running, info.places)}
            step={stepText(running, info.places)}
            waiting={d.preparing}
          />
        </GroupPanel>
      )}
      {(restored || info.pendingRestoreAtMs != null) && (
        <GroupPanel>
          <Banner
            title={d.pending(dateTime(restored?.madeAtMs ?? info.pendingRestoreAtMs ?? 0))}
            body={d.restart}
          />
        </GroupPanel>
      )}
      {(info.enabled || info.places.length > 0) && (
        <li className="oik-group__item">
          <div className="screen-backup-actions">
            {info.enabled && (
              <>
                <Button variant="primary" disabled={busy} onClick={() => void run(() => client.backUpNow())}>
                  {d.backUpNow}
                </Button>
                <Button
                  disabled={busy || info.places.every((p) => p.lastBackup === null)}
                  onClick={() => void run(() => client.checkBackup())}
                >
                  {d.check}
                </Button>
                <Button disabled={busy} onClick={() => setEditing(true)}>
                  {d.change}
                </Button>
              </>
            )}
            <Button variant="quiet" disabled={busy} onClick={() => setRestoring(true)}>
              {d.restore}
            </Button>
          </div>
        </li>
      )}
      {act.error && (
        <GroupPanel>
          <ErrorText>{act.error}</ErrorText>
        </GroupPanel>
      )}
      {editing && (
        <BackupSetup
          info={info}
          dataDir={dataDir}
          onClose={() => setEditing(false)}
          onSaved={(updated) => {
            toast(
              info.enabled
                ? d.saved
                : updated.nextAtMs != null
                  ? d.turnedOn(dayAndTime(updated.nextAtMs))
                  : d.saved,
            );
            setInfo(updated);
            setEditing(false);
            act.clear();
            storages.reload();
          }}
        />
      )}
      {shown && listing !== undefined && (
        <BackupList index={listing} place={shown} kept={kept} onClose={() => setListing(undefined)} />
      )}
      {restoring && (
        <RestoreDialog
          places={info.places}
          onClose={() => setRestoring(false)}
          onRestored={(result) => setRestored(result)}
        />
      )}
    </Group>
  );
}

/** A place in the setup dialog: this computer ("") or a storage, and the folder chosen there. */
type Draft = { id: number; place: string; path: string };

/**
 * Turning backups on, or changing them: where they go (one to four places)
 * and when. Opened again later, it starts from what was chosen.
 */
function BackupSetup({
  info,
  dataDir,
  onClose,
  onSaved,
}: {
  info: BackupInfo;
  dataDir: string;
  onClose: () => void;
  onSaved: (info: BackupInfo) => void;
}) {
  const client = useClient();
  const storages = useLoad(() => client.listStorages(), []);
  const save = useAction();
  const [drafts, setDrafts] = useState<Draft[]>(() =>
    info.places.length > 0
      ? info.places.map((p, i) => ({ id: i, place: p.place ?? "", path: p.path }))
      : [{ id: 0, place: "", path: "" }],
  );
  const [nextId, setNextId] = useState(drafts.length);
  const [every, setEvery] = useState<BackupEvery>(info.schedule?.every ?? "daily");
  const [weekday, setWeekday] = useState(info.schedule?.weekday ?? 0);
  const [time, setTime] = useState(info.schedule ? hhmm(info.schedule.hour, info.schedule.minute) : "03:00");
  const [browsing, setBrowsing] = useState<number>();

  const eligible = (storages.data ?? []).filter((x) => x.name !== null && BACKUP_KINDS.includes(x.type));
  const storageLabel = (name: string) =>
    eligible.find((x) => x.name === name)?.label ?? info.places.find((p) => p.place === name)?.label ?? name;
  const options = (draft: Draft) => [
    { value: "", label: d.thisComputer },
    ...eligible.map((x) => ({ value: x.name!, label: x.label })),
    // A storage chosen before that is no longer listed stays, as it was.
    ...(draft.place && !eligible.some((x) => x.name === draft.place)
      ? [{ value: draft.place, label: storageLabel(draft.place) }]
      : []),
  ];
  // What the node would refuse, said before asking it.
  const problem = (draft: Draft): string | undefined => {
    if (draft.place) return draft.path.split("/").some((part) => part === "..") ? d.badWithin : undefined;
    const overlaps = draft.path !== "" && (inside(draft.path, dataDir) || inside(dataDir, draft.path));
    return overlaps ? d.insideData : undefined;
  };
  const ready = drafts.every((draft) => (draft.place || draft.path) && !problem(draft));
  const update = (id: number, change: Partial<Draft>) => {
    setDrafts((list) => list.map((draft) => (draft.id === id ? { ...draft, ...change } : draft)));
    save.clear();
  };

  // Every half hour, and the time chosen before if it is between them.
  const times = Array.from({ length: 48 }, (_, i) => hhmm(Math.floor(i / 2), (i % 2) * 30));
  if (!times.includes(time)) times.push(time);
  times.sort();

  const submit = async () => {
    const [hour = 3, minute = 0] = time.split(":").map(Number);
    const updated = await save.run(
      () =>
        client.setBackup({
          destinations: drafts.map((draft) =>
            draft.place ? { place: draft.place, path: draft.path.trim() } : { path: draft.path.trim() },
          ),
          schedule: every === "weekly" ? { every, weekday, hour, minute } : { every, hour, minute },
        }),
      asSentence,
    );
    if (updated) onSaved(updated);
  };

  const browsed = drafts.find((draft) => draft.id === browsing);
  return (
    <>
      <Dialog
        title={info.enabled ? d.changeTitle : d.setupTitle}
        closeLabel={strings.common.close}
        onClose={onClose}
        placement="top"
        footer={
          <>
            <Button onClick={onClose}>{strings.common.cancel}</Button>
            <Button variant="primary" disabled={!ready || save.busy} onClick={() => void submit()}>
              {info.enabled ? strings.common.save : d.turnOnAction}
            </Button>
          </>
        }
      >
        <p className="oik-dialog__text">{d.what}</p>
        <div className="screen-backup-places">
          {drafts.map((draft, i) => {
            const label = draft.place ? storageLabel(draft.place) : d.thisComputer;
            const where = draft.place
              ? [label, draft.path.replace(/^\/+|\/+$/g, ""), REPOSITORY_FOLDER].filter(Boolean).join(" / ")
              : draft.path
                ? homeRelative(`${draft.path.replace(/[\\/]+$/, "")}/${REPOSITORY_FOLDER}`)
                : undefined;
            const wrong = problem(draft);
            return (
              <div key={draft.id} className="screen-backup-place">
                <div className="screen-backup-place__head">
                  <Select
                    label={drafts.length === 1 ? d.place : d.placeN(i + 1)}
                    value={draft.place}
                    onChange={(value) => update(draft.id, { place: value, path: "" })}
                    options={options(draft)}
                  />
                  {drafts.length > 1 && (
                    <IconButton
                      icon="close"
                      label={d.removePlace(label)}
                      onClick={() => {
                        setDrafts((list) => list.filter((x) => x.id !== draft.id));
                        save.clear();
                      }}
                    />
                  )}
                </div>
                <div className="oik-field">
                  <span className="oik-field__label">{draft.place ? d.folderWithin : d.folder}</span>
                  <div className="screen-source-pick">
                    <span className="screen-path" translate="no" title={draft.path}>
                      {draft.place
                        ? draft.path
                          ? placePath(draft)
                          : d.topLevel
                        : draft.path
                          ? homeRelative(draft.path)
                          : d.noFolder}
                    </span>
                    <Button onClick={() => setBrowsing(draft.id)}>{d.choose}</Button>
                  </div>
                </div>
                {wrong ? (
                  <ErrorText>{wrong}</ErrorText>
                ) : (
                  where && (
                    <p className="screen-help" translate="no">
                      {d.keptIn(where)}
                      {!draft.place && onOwnDisk(draft.path) && ` ${d.ownDisk}`}
                    </p>
                  )
                )}
              </div>
            );
          })}
          {drafts.length < MAX_PLACES && (
            <div>
              <Button
                variant="quiet"
                icon="plus"
                onClick={() => {
                  setDrafts((list) => [...list, { id: nextId, place: "", path: "" }]);
                  setNextId((n) => n + 1);
                  save.clear();
                }}
              >
                {d.addPlace}
              </Button>
            </div>
          )}
        </div>
        <div className="screen-backup-when">
          <div className="oik-field">
            <span className="oik-field__label">{d.when}</span>
            <Segmented
              label={d.when}
              value={every}
              onChange={(value) => setEvery(value === "weekly" ? "weekly" : "daily")}
              options={[
                { value: "daily", label: d.daily },
                { value: "weekly", label: d.weekly },
              ]}
            />
          </div>
          {every === "weekly" && (
            <Select
              label={d.day}
              value={String(weekday)}
              onChange={(value) => setWeekday(Number(value))}
              options={Array.from({ length: 7 }, (_, day) => ({ value: String(day), label: capitalized(weekdayName(day)) }))}
            />
          )}
          <Select
            label={d.time}
            value={time}
            onChange={setTime}
            options={times.map((value) => {
              const [hour = 0, minute = 0] = value.split(":").map(Number);
              return { value, label: clock(hour, minute) };
            })}
          />
        </div>
        <p className="screen-help">{d.timing}</p>
        {save.error && <ErrorText>{save.error}</ErrorText>}
      </Dialog>
      {browsed && (
        <FolderBrowser
          title={strings.storage.browseTitle}
          place={browsed.place ? { name: browsed.place, label: storageLabel(browsed.place) } : undefined}
          hint={(here) =>
            d.keptIn(
              browsed.place
                ? [storageLabel(browsed.place), here.path, REPOSITORY_FOLDER].filter(Boolean).join(" / ")
                : homeRelative(`${here.path.replace(/[\\/]+$/, "")}/${REPOSITORY_FOLDER}`),
            )
          }
          onClose={() => setBrowsing(undefined)}
          onChoose={(picked) => {
            update(browsed.id, { path: picked });
            setBrowsing(undefined);
          }}
        />
      )}
    </>
  );
}

/** While the backups at a place are opened, which takes a moment: their key is worked out anew each time. */
function Opening() {
  return (
    <div className="screen-backup-busy" role="status">
      <span className="screen-progress" aria-hidden="true">
        <i />
      </span>
      <p className="screen-help">{d.opening}</p>
    </div>
  );
}

/** The backups kept at a place, the newest first. */
function BackupList({
  index,
  place,
  kept,
  onClose,
}: {
  index: number;
  place: BackupPlace;
  kept: string;
  onClose: () => void;
}) {
  const client = useClient();
  const list = useLoad(() => client.listBackups({ place: index }), [index], asSentence);
  return (
    <Dialog
      title={d.listTitle(placeLabel(place))}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <Button variant="primary" onClick={onClose}>
          {strings.common.done}
        </Button>
      }
    >
      <p className="screen-path" translate="no">
        {placePath(place)}
      </p>
      {list.error && <ErrorText>{list.error}</ErrorText>}
      {!list.data && !list.error && <Opening />}
      {list.data?.length === 0 && <p className="oik-dialog__text">{d.noneYet}</p>}
      {list.data && list.data.length > 0 && (
        <ul className="screen-backup-list" aria-label={d.listTitle(placeLabel(place))}>
          {list.data.map((item) => (
            <li key={item.id}>
              <span className="screen-backup-list__when">{dateTime(item.madeAtMs)}</span>
              <span className="screen-backup-list__meta">
                {item.thisFolder
                  ? d.sizeAdded(bytes(item.bytes), bytes(item.added))
                  : `${d.sizeAdded(bytes(item.bytes), bytes(item.added))} · ${d.otherFolder}`}
              </span>
            </li>
          ))}
        </ul>
      )}
      <p className="screen-help">{kept}</p>
    </Dialog>
  );
}

/**
 * Restore from Backup: where the backups are (a backup place, or another
 * folder on this computer), which backup, a clear word on what it replaces,
 * the recovery key when this data center's identity no longer opens them,
 * then what the restore found. It takes effect at Oikome's next start.
 */
function RestoreDialog({
  places,
  onClose,
  onRestored,
}: {
  places: BackupPlace[];
  onClose: () => void;
  onRestored: (result: RestoredBackup) => void;
}) {
  const client = useClient();
  const restore = useAction();
  const [from, setFrom] = useState(places.length > 0 ? "0" : ANOTHER_FOLDER);
  const [folder, setFolder] = useState("");
  const [chosen, setChosen] = useState<string>();
  const [step, setStep] = useState<"choose" | "confirm" | "done">("choose");
  const [needsKey, setNeedsKey] = useState(false);
  const [key, setKey] = useState("");
  const [browsing, setBrowsing] = useState(false);
  const [result, setResult] = useState<RestoredBackup>();
  const index = from === ANOTHER_FOLDER ? undefined : Number(from);
  const list = useLoad(
    () => (index === undefined ? Promise.resolve<BackupItem[]>([]) : client.listBackups({ place: index })),
    [index],
    asSentence,
  );
  // Another place's list is not shown while this one's loads.
  const items = index === undefined || list.loading ? [] : (list.data ?? []);
  // The newest backup of this data folder, until another is chosen.
  const usable = items.filter((item) => item.thisFolder);
  const picked = usable.find((item) => item.id === chosen) ?? usable[0];
  const ready = index === undefined ? folder !== "" : picked !== undefined;

  const close = () => {
    if (!restore.busy) onClose();
  };
  const start = async () => {
    let reason: string | undefined;
    const done = await restore.run(
      () =>
        client.restoreFromBackup({
          ...(index === undefined ? { folder } : { place: index, backup: picked?.id }),
          ...(needsKey && key.trim() ? { recoveryKey: key.trim() } : {}),
        }),
      (e) => {
        reason = e.reason;
        return asSentence(e);
      },
    );
    if (done) {
      setResult(done);
      setStep("done");
      onRestored(done);
    } else if (reason === "backup_key_needed") {
      setNeedsKey(true);
    }
  };

  let body;
  let footer;
  if (step === "done" && result) {
    body = (
      <>
        <p className="oik-dialog__text">{d.ready(dateTime(result.madeAtMs), result.files)}</p>
        {result.missingOriginals.length > 0 && (
          <section className="screen-backup-missing" aria-label={d.missingTitle}>
            <h3 className="oik-label">{d.missingTitle}</h3>
            <ul>
              {result.missingOriginals.map((m) => (
                <li key={`${m.collection}:${m.folder}`}>
                  <span>{d.missing(m.collection, m.missing, homeRelative(m.folder))}</span>
                  {m.some.length > 0 && (
                    <span className="screen-path" translate="no">
                      {d.missingSome(`${listNames(m.some.slice(0, 5))}${m.missing > Math.min(5, m.some.length) ? "…" : ""}`)}
                    </span>
                  )}
                </li>
              ))}
            </ul>
          </section>
        )}
        <p className="screen-backup-restart">{d.restart}</p>
      </>
    );
    footer = (
      <Button variant="primary" onClick={onClose}>
        {strings.common.done}
      </Button>
    );
  } else if (step === "confirm") {
    body = (
      <>
        <p className="oik-dialog__text">
          {index === undefined || !picked ? d.replacesNewest(homeRelative(folder)) : d.replaces(dateTime(picked.madeAtMs))}
        </p>
        <p className="oik-dialog__text">
          {d.originals} {d.atNextStart}
        </p>
        {needsKey && (
          <TextArea label={d.keyLabel} name="recovery-key" hint={d.keyHint} value={key} onChange={setKey} mono autoFocus />
        )}
        {restore.busy && (
          <div className="screen-backup-busy">
            <span className="screen-progress" role="progressbar" aria-label={d.restoring}>
              <i />
            </span>
            <p className="screen-help">{d.restoringBody}</p>
          </div>
        )}
        {restore.error && <ErrorText>{restore.error}</ErrorText>}
      </>
    );
    footer = (
      <>
        <Button
          disabled={restore.busy}
          onClick={() => {
            setStep("choose");
            restore.clear();
          }}
        >
          {d.back}
        </Button>
        <Button
          variant="destructive"
          disabled={restore.busy || (needsKey && !key.trim())}
          onClick={() => void start()}
        >
          {restore.busy ? d.restoring : d.restoreAction}
        </Button>
      </>
    );
  } else {
    body = (
      <>
        <p className="oik-dialog__text">{d.restoreBody}</p>
        <Select
          label={d.restoreFrom}
          value={from}
          onChange={(value) => {
            setFrom(value);
            setChosen(undefined);
            setNeedsKey(false);
            setKey("");
            restore.clear();
          }}
          options={[
            ...places.map((place, i) => ({ value: String(i), label: `${placeLabel(place)} · ${placePath(place)}` })),
            { value: ANOTHER_FOLDER, label: d.anotherFolder },
          ]}
        />
        {index === undefined ? (
          <div className="oik-field">
            <span className="oik-field__label">{d.folder}</span>
            <div className="screen-source-pick">
              <span className="screen-path" translate="no" title={folder}>
                {folder ? homeRelative(folder) : d.noFolder}
              </span>
              <Button onClick={() => setBrowsing(true)}>{d.choose}</Button>
            </div>
            <span className="oik-field__hint">{d.anotherHint}</span>
          </div>
        ) : list.error ? (
          <ErrorText>{list.error}</ErrorText>
        ) : !list.data || list.loading ? (
          <Opening />
        ) : items.length === 0 ? (
          <p className="oik-dialog__text">{d.noneYet}</p>
        ) : (
          <>
            <ChoiceList
              label={d.whichBackup}
              name="restore-backup"
              value={picked?.id ?? ""}
              onChange={setChosen}
              choices={items.map((item) => ({
                value: item.id,
                title: dateTime(item.madeAtMs),
                meta: item.thisFolder
                  ? d.sizeAdded(bytes(item.bytes), bytes(item.added))
                  : `${d.sizeAdded(bytes(item.bytes), bytes(item.added))} · ${d.otherFolder}`,
                disabled: !item.thisFolder,
              }))}
            />
            {usable.length < items.length && <p className="screen-help">{d.elsewhere}</p>}
          </>
        )}
      </>
    );
    footer = (
      <>
        <Button onClick={close}>{strings.common.cancel}</Button>
        <Button variant="primary" disabled={!ready} onClick={() => setStep("confirm")}>
          {d.continue}
        </Button>
      </>
    );
  }

  return (
    <>
      <Dialog title={d.restoreTitle} closeLabel={strings.common.close} onClose={close} placement="top" footer={footer}>
        {body}
      </Dialog>
      {browsing && (
        <FolderBrowser
          title={strings.storage.browseTitle}
          hint={() => d.anotherHint}
          onClose={() => setBrowsing(false)}
          onChoose={(picked) => {
            setFolder(picked);
            setBrowsing(false);
            restore.clear();
            setNeedsKey(false);
          }}
        />
      )}
    </>
  );
}

