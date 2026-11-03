// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Where collections are kept: this computer (in its data folder, or a
// folder the owner picks), a cloud drive or a drive found on it, or a NAS.
// A web page cannot ask the system for a folder, so the node lists them and
// a folder browser picks one, as NAS and media server apps do.

import { useState } from "react";
import type { ReactNode } from "react";
import { Button, Checkbox, ChoiceList, Dialog, ErrorText, Icon, IconButton, TextField } from "@oikome/ui";
import type { Choice, IconName } from "@oikome/ui";
import type { FolderListing, NewStorage, StorageSummary } from "@oikome/client";
import { useClient, useLoad } from "./data";
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

/** What a picked place becomes: nothing to add (this computer), or a storage. */
export type Plan = { add?: Omit<NewStorage, "makeDefault"> } | { error: string };

/** The choices and their fields; `plan()` says what to add. */
export function useStoragePicker({ withThisComputer }: { withThisComputer: boolean }) {
  const client = useClient();
  const info = useLoad(() => client.nodeInfo(), []);
  const places = useLoad(() => client.findPlaces(), []);
  const [choice, setChoice] = useState(withThisComputer ? "here" : "folder");
  // A folder on this computer the owner picked instead of the data folder.
  const [hereFolder, setHereFolder] = useState<string>();
  const [otherFolder, setOtherFolder] = useState<string>();
  const [browsing, setBrowsing] = useState<"here" | "folder">();
  const [address, setAddress] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [encrypt, setEncrypt] = useState(true);

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
  return { picker, plan };
}

/** Add what `plan` says, if anything; the storage, or undefined for this computer. */
export async function addPlanned(
  client: ReturnType<typeof useClient>,
  plan: Plan,
  makeDefault: boolean,
): Promise<StorageSummary | undefined> {
  if ("error" in plan) throw new Error(plan.error);
  if (!plan.add) return undefined;
  return client.addStorage({ ...plan.add, makeDefault });
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
