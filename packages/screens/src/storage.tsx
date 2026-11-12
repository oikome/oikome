// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Where collections are kept: this computer (in its data folder, a folder
// the owner picks, or a drive plugged in), a NAS, a cloud drive (its
// desktop app's folder, or Google Drive and Dropbox signed in to on their
// pages, through an app the owner registers), or object storage
// (RustFS).
// A web page cannot ask the system for a folder, so the node lists them and
// a folder browser picks one, as NAS and media server apps do.

import { useState } from "react";
import type { ReactNode } from "react";
import { Button, Checkbox, ChoiceList, Dialog, ErrorText, Icon, IconButton, InsetWell, Select, TextField } from "@oikome/ui";
import type { Choice } from "@oikome/ui";
import type { DropboxSignIn, FolderListing, GoogleReturn, NewStorage, StartDropbox, StorageSummary } from "@oikome/client";
import { useAction, useClient, useLoad } from "./data";
import { homeRelative } from "./format";
import { strings } from "./strings";

const s = strings.storage;
const DATA_FOLDER = "Oikome";

/** The folder Oikome keeps its files in for a picked one, as the node does. */
function keptIn(picked: string): string {
  const name = picked.split(/[\\/]/).pop();
  return name === DATA_FOLDER ? picked : `${picked.replace(/[\\/]+$/, "")}/${DATA_FOLDER}`;
}

function folderName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/**
 * What a picked place becomes: nothing to add (this computer), a storage,
 * or Google Drive, connected on Google's page.
 */
export type Plan =
  | { add?: Omit<NewStorage, "makeDefault"> }
  // Without an app: the one the node remembers.
  | { google: { clientId?: string; clientSecret?: string; readAll?: boolean } }
  | { dropbox: { clientId?: string; readAll?: boolean } }
  | { error: string };

/** The Client ID and Client secret in an OAuth client's downloaded JSON. */
async function googleAppFile(file: File): Promise<{ clientId: string; clientSecret: string } | undefined> {
  try {
    const json = JSON.parse(await file.text());
    const app = json.installed ?? json.web ?? json;
    if (typeof app.client_id === "string" && typeof app.client_secret === "string") {
      return { clientId: app.client_id, clientSecret: app.client_secret };
    }
  } catch {
    // Not JSON: said below.
  }
  return undefined;
}

/** Where the owner registers their Google app, step by step. */
const GOOGLE_STEPS: [string, () => string][] = [
  ["https://console.cloud.google.com/projectcreate", () => s.gdriveStep1],
  ["https://console.cloud.google.com/apis/library/drive.googleapis.com", () => s.gdriveStep2],
  ["https://console.cloud.google.com/auth/branding", () => s.gdriveStep3],
  ["https://console.cloud.google.com/auth/clients/create", () => s.gdriveStep4],
];

/** Google Drive's name in the places found here, as the node gives it. */
const GOOGLE_DRIVE = "Google Drive";
const DROPBOX = "Dropbox";
const ICLOUD = "iCloud Drive";
const ONEDRIVE = "OneDrive";
/**
 * What the owner ticks on a Dropbox app's Permissions tab, in the tab's own
 * order. The console adds account_info.read and files.metadata.read itself,
 * greyed out, so they aren't listed.
 */
const DROPBOX_TICK = ["files.metadata.write", "files.content.write", "files.content.read"];
/** The owner's app's Permissions tab, once its App key is known. */
const dropboxPermissions = (appKey: string) =>
  appKey ? `https://www.dropbox.com/developers/console/apps/${encodeURIComponent(appKey)}/permissions` : undefined;

/**
 * A cloud drive: its desktop app's folder, where the app is on this
 * computer (nothing to register), and whether Oikome can sign in to it on
 * its page instead (an app of the owner's own, registered once).
 */
interface Drive {
  label: string;
  sync?: string;
  signIn: boolean;
}

/** Each drive by the name its maker gives it in the language in use. */
function driveName(label: string): string {
  return label === GOOGLE_DRIVE ? s.gdrive : label === ICLOUD ? s.icloud : label;
}

/** The choices and their fields; `plan()` says what to add. */
export function useStoragePicker({ withThisComputer }: { withThisComputer: boolean }) {
  const client = useClient();
  const info = useLoad(() => client.nodeInfo(), []);
  const places = useLoad(() => client.findPlaces(), []);
  // The Google app remembered from the last Google Drive connected.
  const googleApp = useLoad(() => client.googleApp(), []);
  const dropboxApp = useLoad(() => client.dropboxApp(), []);
  const [otherApp, setOtherApp] = useState(false);
  const [appFileError, setAppFileError] = useState<string>();
  const [appFileName, setAppFileName] = useState<string>();
  // What kind of place: this computer or a folder, a NAS, a cloud drive, object storage.
  const [choice, setChoice] = useState(withThisComputer ? "here" : "folder");
  // A folder on this computer the owner picked instead of the data folder.
  const [hereFolder, setHereFolder] = useState<string>();
  const [otherFolder, setOtherFolder] = useState<string>();
  const [browsing, setBrowsing] = useState<"here" | "folder">();
  // Which cloud drive, and how it connects.
  const [drivePick, setDrivePick] = useState(GOOGLE_DRIVE);
  const [how, setHow] = useState<"sync" | "signIn">("sync");
  const [address, setAddress] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [encrypt, setEncrypt] = useState(true);
  // Object storage: its own fields, encrypted unless the owner says no.
  const [s3Address, setS3Address] = useState("");
  const [bucket, setBucket] = useState("");
  const [accessKey, setAccessKey] = useState("");
  const [secretKey, setSecretKey] = useState("");
  const [s3Encrypt, setS3Encrypt] = useState(true);
  // Google Drive: the owner's own Google app.
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [dropboxClientId, setDropboxClientId] = useState("");
  // Also read the rest of the drive, to view folders there in place.
  const [readAll, setReadAll] = useState(false);

  const found = places.data ?? [];
  // API drives can always be signed in to. Sync-only drives stay visible
  // before their desktop folder is found so the owner can see how to set one up.
  const drives: Drive[] = found
    .filter((place) => place.type === "drive")
    .map((place) => ({
      label: place.label,
      sync: place.path,
      signIn: place.label === GOOGLE_DRIVE || place.label === DROPBOX,
    }));
  for (const [label, signIn] of [
    [GOOGLE_DRIVE, true],
    [DROPBOX, true],
    [ICLOUD, false],
    [ONEDRIVE, false],
  ] as const) {
    if (!drives.some((d) => d.label === label)) drives.push({ label, signIn });
  }
  drives.sort((a, b) => Number(b.label === GOOGLE_DRIVE) - Number(a.label === GOOGLE_DRIVE));
  const drive = drives.find((d) => d.label === drivePick) ?? drives[0]!;
  const driveHow = drive.sync && drive.signIn ? how : drive.sync ? "sync" : "signIn";

  const kinds: Choice[] = [
    withThisComputer
      ? {
          value: "here",
          title: s.thisComputer,
          meta: homeRelative(hereFolder ? keptIn(hereFolder) : (info.data?.dataDir ?? "…")),
          leading: <Icon name="computer" />,
          aside: (
            <Button
              variant="quiet"
              onClick={() => {
                setChoice("here");
                setBrowsing("here");
              }}
            >
              {s.change}
            </Button>
          ),
        }
      : { value: "folder", title: s.folder, meta: s.folderMeta, leading: <Icon name="computer" /> },
    { value: "nas", title: s.nas, meta: s.nasMeta, leading: <Icon name="nas" /> },
    { value: "drive", title: s.drive, meta: s.driveMeta, leading: <Icon name="drive" /> },
    { value: "s3", title: s.s3, meta: s.s3Meta, leading: <Icon name="bucket" /> },
  ];

  // The app the node remembers, unless the owner picks another one.
  const rememberedApp = otherApp ? undefined : (googleApp.data?.clientId ?? undefined);
  const rememberedDropboxApp = otherApp ? undefined : (dropboxApp.data?.clientId ?? undefined);
  // The app's two fields filled from the file Google lets one download.
  const fillFromFile = async (file: File | undefined) => {
    if (!file) return;
    setAppFileName(file.name);
    const app = await googleAppFile(file);
    setAppFileError(app ? undefined : s.googleFileUnreadable);
    if (app) {
      setClientId(app.clientId);
      setClientSecret(app.clientSecret);
    }
  };
  // Registering the app, then giving it to Oikome: two steps, each its own group.
  const googleAppFields = (
    <div className="screen-gapp">
      {!drive.sync && <p className="screen-help">{s.gdriveEasier}</p>}
      <section className="screen-gapp__step" aria-labelledby="gapp-register">
        <h3 id="gapp-register" className="screen-gapp__title">
          <span className="screen-gapp__n" aria-hidden="true">
            1
          </span>
          {s.gdriveRegister}
        </h3>
        <p className="screen-help">{s.gdriveRegisterHint}</p>
        <ol className="screen-steps">
          {GOOGLE_STEPS.map(([href, step]) => (
            <li key={href}>
              <a href={href} target="_blank" rel="noreferrer">
                {step()}
              </a>
            </li>
          ))}
        </ol>
      </section>
      <section className="screen-gapp__step" aria-labelledby="gapp-give">
        <h3 id="gapp-give" className="screen-gapp__title">
          <span className="screen-gapp__n" aria-hidden="true">
            2
          </span>
          {s.gdriveGive}
        </h3>
        <div className="screen-gapp__file">
          <label className="oik-button oik-button--secondary screen-file-pick">
            <Icon name="upload" />
            {s.fillFromFile}
            <input
              type="file"
              accept="application/json,.json"
              onChange={(e) => {
                void fillFromFile(e.target.files?.[0]);
                e.target.value = "";
              }}
            />
          </label>
          <span className={appFileError ? "screen-help screen-help--error" : "screen-help"} role={appFileError ? "alert" : undefined}>
            {appFileError ?? appFileName ?? s.googleFileHint}
          </span>
        </div>
        <p className="screen-gapp__or">{s.gdriveOrPaste}</p>
        <TextField label={s.clientId} name="google-client-id" value={clientId} onChange={setClientId} />
        <TextField
          label={s.clientSecret}
          name="google-client-secret"
          type="password"
          hint={s.passwordHint}
          value={clientSecret}
          onChange={setClientSecret}
        />
      </section>
    </div>
  );

  // Registering the app, then giving Oikome its key: no redirect URI to
  // register. Once the App key is in, Permissions links to the app's own tab.
  const permissionsTab = dropboxPermissions(dropboxClientId.trim());
  const toTab = (href: string | undefined, text: string) =>
    href ? (
      <a href={href} target="_blank" rel="noreferrer">
        {text}
      </a>
    ) : (
      text
    );
  const dropboxAppFields = (
    <div className="screen-gapp">
      {!drive.sync && <p className="screen-help">{s.dropboxEasier}</p>}
      <section className="screen-gapp__step" aria-labelledby="dropbox-register">
        <h3 id="dropbox-register" className="screen-gapp__title">
          <span className="screen-gapp__n" aria-hidden="true">
            1
          </span>
          {s.dropboxRegister}
        </h3>
        <p className="screen-help">{s.dropboxRegisterHint}</p>
        <ol className="screen-steps">
          <li>
            <a href="https://www.dropbox.com/developers/apps/create" target="_blank" rel="noreferrer">
              {s.dropboxCreate}
            </a>
            {s.dropboxCreateType}
          </li>
          <li>
            {toTab(permissionsTab, s.dropboxPermissions)}
            <ul className="screen-steps__values" translate="no">
              {DROPBOX_TICK.map((scope) => (
                <li key={scope}>{scope}</li>
              ))}
            </ul>
          </li>
        </ol>
      </section>
      <section className="screen-gapp__step" aria-labelledby="dropbox-give">
        <h3 id="dropbox-give" className="screen-gapp__title">
          <span className="screen-gapp__n" aria-hidden="true">
            2
          </span>
          {s.gdriveGive}
        </h3>
        <TextField
          label={s.dropboxAppKey}
          name="dropbox-client-id"
          hint={s.dropboxAppKeyHint}
          value={dropboxClientId}
          onChange={setDropboxClientId}
        />
      </section>
    </div>
  );

  let form: ReactNode = null;
  if (choice === "folder") {
    // The owner picks the folder: on this computer, or on a drive plugged in.
    form = (
      <div className="oik-field">
        <span className="oik-field__label">{s.whichFolder}</span>
        <div className="screen-folder-pick">
          <Icon name="folder" />
          {otherFolder ? (
            <span className="screen-path" translate="no" title={otherFolder}>
              {homeRelative(otherFolder)}
            </span>
          ) : (
            <span className="screen-folder-pick__none">{s.noFolder}</span>
          )}
          <Button onClick={() => setBrowsing("folder")}>{otherFolder ? s.change : s.chooseFolder}</Button>
        </div>
        <span className="oik-field__hint">
          {otherFolder ? s.willKeep(homeRelative(keptIn(otherFolder))) : s.makesFolder}
        </span>
      </div>
    );
  } else if (choice === "nas") {
    form = (
      <>
        <TextField
          label={s.address}
          name="nas-address"
          type="url"
          hint={s.addressHint}
          value={address}
          onChange={setAddress}
        />
        <TextField label={s.username} name="nas-user" value={username} onChange={setUsername} />
        <TextField
          label={s.password}
          name="nas-password"
          type="password"
          hint={s.passwordHint}
          value={password}
          onChange={setPassword}
        />
        <Checkbox label={s.encrypt} checked={encrypt} onChange={setEncrypt} />
      </>
    );
  } else if (choice === "drive") {
    form = (
      <>
        <Select
          label={s.whichDrive}
          value={drive.label}
          onChange={setDrivePick}
          options={drives.map((d) => ({ value: d.label, label: driveName(d.label) }))}
        />
        {drive.sync && drive.signIn && (
          <ChoiceList
            label={s.howConnects}
            name="how-connects"
            value={driveHow}
            onChange={(value) => setHow(value as "sync" | "signIn")}
            choices={[
              { value: "sync", title: s.howSync, meta: s.howSyncMeta(homeRelative(drive.sync)) },
              { value: "signIn", title: s.howSignIn, meta: s.howSignInMeta },
            ]}
          />
        )}
        {driveHow === "sync" && drive.sync ? (
          <InsetWell>
            <p className="screen-well-text">
              {s.syncKept(homeRelative(keptIn(drive.sync)))} {s.driveEncrypted(driveName(drive.label))}
            </p>
          </InsetWell>
        ) : !drive.sync && !drive.signIn ? (
          <InsetWell>
            <p className="screen-well-text">
              <strong>{s.driveNotFound}</strong>
            </p>
            <p className="screen-well-text">{drive.label === ICLOUD ? s.icloudSetup : s.oneDriveSetup}</p>
            <Button variant="quiet" onClick={places.reload} disabled={places.loading}>
              {places.loading ? s.checking : s.checkAgain}
            </Button>
          </InsetWell>
        ) : (
          <>
            {drive.label === DROPBOX ? (
              rememberedDropboxApp ? (
                <div className="screen-app-row">
                  <span className="screen-help screen-ellipsis" translate="no">
                    {s.dropboxApp(rememberedDropboxApp)}
                  </span>
                  <Button variant="quiet" onClick={() => setOtherApp(true)}>
                    {s.otherApp}
                  </Button>
                </div>
              ) : (
                dropboxAppFields
              )
            ) : rememberedApp ? (
              <div className="screen-app-row">
                <span className="screen-help screen-ellipsis" translate="no">
                  {s.gdriveApp(rememberedApp)}
                </span>
                <Button variant="quiet" onClick={() => setOtherApp(true)}>
                  {s.otherApp}
                </Button>
              </div>
            ) : (
              googleAppFields
            )}
            <Checkbox
              label={drive.label === DROPBOX ? s.dropboxReadAll : s.gdriveReadAll}
              checked={readAll}
              onChange={setReadAll}
            />
            <InsetWell>
              <p className="screen-well-text">
                {drive.label === DROPBOX ? s.dropboxNote(readAll) : s.gdriveNote(readAll)}
              </p>
            </InsetWell>
          </>
        )}
      </>
    );
  } else if (choice === "s3") {
    form = (
      <>
        <TextField
          label={s.address}
          name="s3-address"
          type="url"
          hint={s.s3AddressHint}
          value={s3Address}
          onChange={setS3Address}
        />
        <TextField label={s.bucket} name="s3-bucket" hint={s.bucketHint} value={bucket} onChange={setBucket} />
        <TextField label={s.accessKey} name="s3-access-key" value={accessKey} onChange={setAccessKey} />
        <TextField
          label={s.secretKey}
          name="s3-secret-key"
          type="password"
          hint={s.passwordHint}
          value={secretKey}
          onChange={setSecretKey}
        />
        <Checkbox label={s.encrypt} checked={s3Encrypt} onChange={setS3Encrypt} />
        {!s3Encrypt && <p className="screen-help">{s.plainNote}</p>}
      </>
    );
  }

  const plan = (): Plan => {
    if (choice === "here") {
      return hereFolder ? { add: { label: folderName(hereFolder), type: "folder", address: hereFolder } } : {};
    }
    if (choice === "nas") {
      if (!address.trim()) return { error: s.addressRequired };
      return {
        add: {
          label: s.nasLabel,
          type: "webdav",
          address: address.trim(),
          username: username.trim() || undefined,
          password: password || undefined,
          encrypted: encrypt,
        },
      };
    }
    if (choice === "drive") {
      if (driveHow === "sync" && drive.sync) {
        return { add: { label: drive.label, type: "drive", address: drive.sync } };
      }
      if (!drive.signIn) return { error: drive.label === ICLOUD ? s.icloudSetup : s.oneDriveSetup };
      if (drive.label === DROPBOX) {
        if (rememberedDropboxApp) return { dropbox: { readAll } };
        if (!dropboxClientId.trim()) return { error: s.dropboxClientRequired };
        return { dropbox: { clientId: dropboxClientId.trim(), readAll } };
      }
      if (rememberedApp) return { google: { readAll } };
      if (!clientId.trim() || !clientSecret.trim()) return { error: s.googleClientRequired };
      return { google: { clientId: clientId.trim(), clientSecret: clientSecret.trim(), readAll } };
    }
    if (choice === "s3") {
      if (![s3Address, bucket, accessKey].every((v) => v.trim()) || !secretKey) return { error: s.s3Required };
      return {
        add: {
          label: bucket.trim(),
          type: "s3",
          address: s3Address.trim(),
          bucket: bucket.trim(),
          username: accessKey.trim(),
          password: secretKey,
          encrypted: s3Encrypt,
        },
      };
    }
    if (!otherFolder) return { error: s.pickFolder };
    return { add: { label: folderName(otherFolder), type: "folder", address: otherFolder } };
  };

  const picker = (
    <>
      <ChoiceList label={s.keepOn} name="keep-on" value={choice} onChange={setChoice} choices={kinds} />
      {form && <div className="screen-place-form">{form}</div>}
      {browsing && (
        <FolderBrowser
          onClose={() => setBrowsing(undefined)}
          onChoose={(path) => {
            if (browsing === "here") setHereFolder(path);
            else setOtherFolder(path);
            setBrowsing(undefined);
          }}
        />
      )}
    </>
  );
  // An API drive is added on its own page: the button names the provider.
  return {
    picker,
    plan,
    choice,
    unavailable: choice === "drive" && !drive.sync && !drive.signIn,
    toGoogle: choice === "drive" && driveHow === "signIn" && drive.label === GOOGLE_DRIVE,
    toDropbox: choice === "drive" && driveHow === "signIn" && drive.label === DROPBOX,
  };
}

/** What to say when the place picked couldn't be reached or refused what was entered. */
export function unreachableFor(plan: Plan): string {
  if ("google" in plan) return s.googleFailed;
  if ("dropbox" in plan) return s.dropboxFailed;
  return "add" in plan && plan.add?.type === "s3" ? s.unreachableS3 : s.unreachable;
}

/** The copied-code part of connecting Dropbox, shared by both app flows. */
export function useDropboxCode() {
  const client = useClient();
  const [pending, setPending] = useState<DropboxSignIn>();
  const [code, setCode] = useState("");
  const start = async (request: StartDropbox) => {
    const signIn = await client.startDropbox(request);
    setCode("");
    setPending(signIn);
    window.open(signIn.url, "_blank", "noopener,noreferrer");
  };
  const finish = async () => {
    if (!pending) throw new Error(s.dropboxFailed);
    if (!code.trim()) throw new Error(s.dropboxCodeRequired);
    try {
      return await client.finishDropbox({ id: pending.id, code: code.trim() });
    } finally {
      // A finished or refused attempt consumes its one-time ID. The next
      // submit starts fresh; an empty code never reaches this point.
      setPending(undefined);
      setCode("");
    }
  };
  const fields = pending ? (
    <section className="screen-gapp__step" aria-label={s.dropboxCode}>
      <p className="screen-help">{s.dropboxCodeHint}</p>
      <TextField
        label={s.dropboxCode}
        name="dropbox-authorization-code"
        value={code}
        onChange={setCode}
        autoFocus
      />
      <a href={pending.url} target="_blank" rel="noreferrer">
        {s.dropboxReopen}
      </a>
    </section>
  ) : null;
  return {
    pending,
    hasCode: Boolean(code.trim()),
    fields,
    start,
    finish,
    reset: () => {
      setPending(undefined);
      setCode("");
    },
  };
}

/**
 * Add what `plan` says, if anything; the storage, or undefined for this
 * computer. Google Drive goes to Google's page instead, and comes back to
 * `returnTo`: the promise doesn't settle, so the page stays busy as it goes.
 * Dropbox's copied-code flow is handled by `useDropboxCode` instead.
 */
export async function addPlanned(
  client: ReturnType<typeof useClient>,
  plan: Plan,
  makeDefault: boolean,
  returnTo: GoogleReturn,
): Promise<StorageSummary | undefined> {
  if ("error" in plan) throw new Error(plan.error);
  if ("dropbox" in plan) throw new Error(s.dropboxFailed);
  if ("google" in plan) {
    const { url } = await client.startGoogleDrive({ ...plan.google, label: s.gdrive, makeDefault, returnTo });
    window.location.assign(url);
    return new Promise<never>(() => undefined);
  }
  if (!plan.add) return undefined;
  return client.addStorage({ ...plan.add, makeDefault });
}

/** What to say on coming back from Google's page (`?google=`), if anything. */
export function googleMessage(outcome: string | null): string | undefined {
  if (outcome === "denied") return s.googleDenied;
  if (outcome === "failed") return s.googleFailed;
  return undefined;
}

/**
 * Add a place beside the ones already there: a cloud drive or drive found
 * here, Google Drive, a NAS, RustFS, or a folder. Google Drive comes back
 * to `returnTo` from Google's page.
 */
export function AddStorage({
  returnTo,
  onClose,
  onAdded,
}: {
  returnTo: GoogleReturn;
  onClose: () => void;
  onAdded: (label: string) => void;
}) {
  const client = useClient();
  const storage = useStoragePicker({ withThisComputer: false });
  const dropbox = useDropboxCode();
  const [makeDefault, setMakeDefault] = useState(false);
  const add = useAction();
  const close = () => {
    dropbox.reset();
    onClose();
  };
  return (
    <Dialog
      title={s.add}
      closeLabel={strings.common.close}
      onClose={close}
      placement="top"
      footer={
        <>
          <Button onClick={close}>{strings.common.cancel}</Button>
          <Button type="submit" form="add-storage" variant="primary" disabled={add.busy || storage.unavailable}>
            {add.busy
              ? s.connecting
              : dropbox.pending
                ? s.dropboxConnect
                : storage.toGoogle
                  ? s.toGoogle
                  : storage.toDropbox
                    ? s.toDropbox
                    : s.add}
          </Button>
        </>
      }
    >
      <form
        id="add-storage"
        onSubmit={async (e) => {
          e.preventDefault();
          if (dropbox.pending) {
            if (!dropbox.hasCode) {
              add.fail(s.dropboxCodeRequired);
              return;
            }
            const added = await add.run(
              dropbox.finish,
              (err) => (err.code === "invalid" ? s.dropboxFailed : undefined),
            );
            if (added) onAdded(added.label);
            return;
          }
          const plan = storage.plan();
          if ("error" in plan) {
            add.fail(plan.error);
            return;
          }
          if ("dropbox" in plan) {
            await add.run(
              () => dropbox.start({ ...plan.dropbox, label: DROPBOX, makeDefault }),
              (err) => (err.code === "invalid" ? unreachableFor(plan) : undefined),
            );
            return;
          }
          const added = await add.run(
            () => addPlanned(client, plan, makeDefault, returnTo),
            (err) => (err.code === "invalid" ? unreachableFor(plan) : undefined),
          );
          if (added) onAdded(added.label);
        }}
      >
        {dropbox.pending ? dropbox.fields : storage.picker}
        {!dropbox.pending && returnTo === "settings" && (
          <Checkbox label={s.useForNew} checked={makeDefault} onChange={setMakeDefault} disabled={storage.unavailable} />
        )}
        {add.error && <ErrorText>{add.error}</ErrorText>}
      </form>
    </Dialog>
  );
}

/** Folders of this computer, one level at a time, to pick one. */
/**
 * Browse this computer's folders, as the node lists them (a web page can't
 * open the system's folder picker), and choose one; with `csv`, choose one
 * of the CSV files in a folder instead.
 */
export function FolderBrowser({
  onClose,
  onChoose,
  title = s.browseTitle,
  hint,
  csv = false,
}: {
  onClose: () => void;
  onChoose: (path: string) => void;
  title?: string;
  /** Under the list: what choosing this folder would do. */
  hint?: (here: FolderListing) => ReactNode;
  csv?: boolean;
}) {
  const client = useClient();
  const [path, setPath] = useState<string>();
  const listing = useLoad(() => client.listFolders(path ? { path } : {}), [path]);
  const here = listing.data;
  return (
    <Dialog
      title={title}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          {!csv && (
            <Button variant="primary" disabled={!here} onClick={() => here && onChoose(here.path)}>
              {s.choose}
            </Button>
          )}
        </>
      }
    >
      <div className="screen-browser">
        <div className="screen-browser__where">
          <IconButton
            icon="back"
            label={s.up}
            disabled={!here?.parent}
            onClick={() => here?.parent && setPath(here.parent)}
          />
          <span className="screen-path" translate="no" title={here?.path}>
            {here ? homeRelative(here.path) : "…"}
          </span>
        </div>
        {listing.error && <ErrorText>{listing.error}</ErrorText>}
        <ul className="screen-browser__list" aria-label={title}>
          {!here && !listing.error && <li className="screen-browser__empty oik-skel" aria-hidden="true" />}
          {here && here.folders.length === 0 && (!csv || here.csvFiles.length === 0) && (
            <li className="screen-browser__empty">{csv ? s.noCsv : s.noFolders}</li>
          )}
          {csv &&
            here?.csvFiles.map((file) => (
              <li key={file.path}>
                <button type="button" className="screen-browser__folder" onClick={() => onChoose(file.path)}>
                  <Icon name="table" />
                  <span translate="no">{file.name}</span>
                </button>
              </li>
            ))}
          {here?.folders.map((folder) => (
            <li key={folder.path}>
              <button type="button" className="screen-browser__folder" onClick={() => setPath(folder.path)}>
                <Icon name="folder" />
                <span translate="no">{folder.name}</span>
                <Icon name="chevron" />
              </button>
            </li>
          ))}
        </ul>
        {here && <p className="screen-help">{hint ? hint(here) : s.willKeep(homeRelative(keptIn(here.path)))}</p>}
      </div>
    </Dialog>
  );
}
