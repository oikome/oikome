// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Where collections are kept: this computer (in its data folder, or a
// folder the owner picks), a cloud drive or a drive found on it, Google
// Drive (through an app the owner registers with Google, signed in to on
// Google's page), a NAS, or RustFS.
// A web page cannot ask the system for a folder, so the node lists them and
// a folder browser picks one, as NAS and media server apps do.

import { useState } from "react";
import type { ReactNode } from "react";
import { Button, Checkbox, ChoiceList, Dialog, ErrorText, Icon, IconButton, TextField } from "@oikome/ui";
import type { Choice, IconName } from "@oikome/ui";
import type { FolderListing, GoogleReturn, NewStorage, StorageSummary } from "@oikome/client";
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
  | { error: string };

/** The client ID and secret in a Google app's downloaded file (`client_secret_….json`). */
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
  ["https://console.cloud.google.com/auth/overview", () => s.gdriveStep3],
  ["https://console.cloud.google.com/auth/clients/create", () => s.gdriveStep4],
];

/** The choices and their fields; `plan()` says what to add. */
export function useStoragePicker({ withThisComputer }: { withThisComputer: boolean }) {
  const client = useClient();
  const info = useLoad(() => client.nodeInfo(), []);
  const places = useLoad(() => client.findPlaces(), []);
  // The Google app remembered from the last Google Drive connected.
  const googleApp = useLoad(() => client.googleApp(), []);
  const [otherApp, setOtherApp] = useState(false);
  const [appFileError, setAppFileError] = useState<string>();
  const [choice, setChoice] = useState(withThisComputer ? "here" : "folder");
  // A folder on this computer the owner picked instead of the data folder.
  const [hereFolder, setHereFolder] = useState<string>();
  const [otherFolder, setOtherFolder] = useState<string>();
  const [browsing, setBrowsing] = useState<"here" | "folder">();
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
  // Also read the rest of the drive, to view folders there in place.
  const [readAll, setReadAll] = useState(false);

  const found = places.data ?? [];
  const choices: Choice[] = [];
  if (withThisComputer) {
    choices.push({
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
    });
  }
  found.forEach((place, i) => {
    choices.push({
      value: `found-${i}`,
      title: place.label,
      meta: place.type === "drive" ? s.foundDrive : s.found,
      leading: <Icon name={(place.type === "drive" ? "drive" : "folder") as IconName} />,
    });
  });
  // The app the node remembers, unless the owner picks another one.
  const rememberedApp = otherApp ? undefined : (googleApp.data?.clientId ?? undefined);
  // The app's two fields filled from the file Google lets one download.
  const fillFromFile = async (file: File | undefined) => {
    if (!file) return;
    const app = await googleAppFile(file);
    setAppFileError(app ? undefined : s.googleFileUnreadable);
    if (app) {
      setClientId(app.clientId);
      setClientSecret(app.clientSecret);
    }
  };
  const googleAppFields = (
    <>
      <p className="screen-help">{s.gdriveEasier}</p>
      <p className="screen-help">{s.gdriveSteps}</p>
      <ol className="screen-steps">
        {GOOGLE_STEPS.map(([href, step]) => (
          <li key={href}>
            <a href={href} target="_blank" rel="noreferrer">
              {step()}
            </a>
          </li>
        ))}
      </ol>
      <label className="oik-button oik-button--quiet screen-file-pick">
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
      {appFileError && <p className="screen-help">{appFileError}</p>}
      <TextField label={s.clientId} name="google-client-id" value={clientId} onChange={setClientId} />
      <TextField
        label={s.clientSecret}
        name="google-client-secret"
        type="password"
        hint={s.passwordHint}
        value={clientSecret}
        onChange={setClientSecret}
      />
    </>
  );
  choices.push({
    value: "gdrive",
    title: s.gdrive,
    meta: s.gdriveMeta,
    leading: <Icon name="drive" />,
    more: (
      <>
        {rememberedApp ? (
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
        <Checkbox label={s.gdriveReadAll} checked={readAll} onChange={setReadAll} />
        <p className="screen-help">{s.gdriveNote}</p>
      </>
    ),
  });
  choices.push({
    value: "nas",
    title: s.nas,
    meta: s.nasMeta,
    leading: <Icon name="nas" />,
    more: (
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
    ),
  });
  choices.push({
    value: "s3",
    title: s.s3,
    meta: s.s3Meta,
    leading: <Icon name="bucket" />,
    more: (
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
    ),
  });
  // On the first run Local's Change… picks a folder of this computer; later,
  // another folder (a mounted share, a drive plugged in) is added here.
  if (!withThisComputer) {
    choices.push({
      value: "folder",
      title: s.folder,
      meta: otherFolder ? s.pickedFolder(homeRelative(keptIn(otherFolder))) : s.folderMeta,
      leading: <Icon name="folder" />,
      aside: (
        <Button
          variant="quiet"
          onClick={() => {
            setChoice("folder");
            setBrowsing("folder");
          }}
        >
          {otherFolder ? s.change : s.chooseFolder}
        </Button>
      ),
    });
  }

  const plan = (): Plan => {
    if (choice === "here") {
      return hereFolder ? { add: { label: folderName(hereFolder), type: "folder", address: hereFolder } } : {};
    }
    if (choice.startsWith("found-")) {
      const place = found[Number(choice.slice("found-".length))];
      return place ? { add: { label: place.label, type: place.type, address: place.path } } : {};
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
    if (choice === "gdrive") {
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
      <ChoiceList label={s.keepOn} name="keep-on" value={choice} onChange={setChoice} choices={choices} />
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
  // Google Drive is added on Google's page: the button says so.
  return { picker, plan, choice, toGoogle: choice === "gdrive" };
}

/** What to say when the place picked couldn't be reached or refused what was entered. */
export function unreachableFor(plan: Plan): string {
  if ("google" in plan) return s.googleFailed;
  return "add" in plan && plan.add?.type === "s3" ? s.unreachableS3 : s.unreachable;
}

/**
 * Add what `plan` says, if anything; the storage, or undefined for this
 * computer. Google Drive goes to Google's page instead, and comes back to
 * `returnTo`: the promise doesn't settle, so the page stays busy as it goes.
 */
export async function addPlanned(
  client: ReturnType<typeof useClient>,
  plan: Plan,
  makeDefault: boolean,
  returnTo: GoogleReturn,
): Promise<StorageSummary | undefined> {
  if ("error" in plan) throw new Error(plan.error);
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
  const [makeDefault, setMakeDefault] = useState(false);
  const add = useAction();
  return (
    <Dialog
      title={s.add}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button type="submit" form="add-storage" variant="primary" disabled={add.busy}>
            {add.busy ? s.connecting : storage.toGoogle ? s.toGoogle : s.add}
          </Button>
        </>
      }
    >
      <form
        id="add-storage"
        onSubmit={async (e) => {
          e.preventDefault();
          const plan = storage.plan();
          if ("error" in plan) {
            add.fail(plan.error);
            return;
          }
          const added = await add.run(
            () => addPlanned(client, plan, makeDefault, returnTo),
            (err) => (err.code === "invalid" ? unreachableFor(plan) : undefined),
          );
          if (added) onAdded(added.label);
        }}
      >
        {storage.picker}
        {returnTo === "settings" && (
          <Checkbox label={s.useForNew} checked={makeDefault} onChange={setMakeDefault} />
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
