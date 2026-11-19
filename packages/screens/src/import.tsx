// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { useEffect, useState } from "react";
import { Banner, Button, ChoiceList, Dialog, ErrorText, Icon, Select, TextField, useToast } from "@oikome/ui";
import type { SpaceSummary } from "@oikome/client";
import { forgetPreviews } from "./content";
import { nameFromFolder } from "./covers";
import { rawMessage, useAction, useClient, useLoad } from "./data";
import { bytes, homeRelative } from "./format";
import { Link, navigate } from "./router";
import { FolderBrowser } from "./storage";
import { strings } from "./strings";

const s = strings.collections;
const NAME = /^[A-Za-z_-][A-Za-z0-9_-]{0,63}$/;
const LOCAL = "";
const NEW = "__new__";
const MAX_IMPORT_BYTES = 32 * 1024 * 1024;

function base64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result !== "string") {
        reject(new Error("The selected file could not be read."));
        return;
      }
      resolve(reader.result.slice(reader.result.indexOf(",") + 1));
    };
    reader.onerror = () => reject(reader.error ?? new Error("The selected file could not be read."));
    reader.readAsDataURL(file);
  });
}

function suggestedName(file: File): string {
  const stem = file.name.replace(/\.[^/.]+$/, "");
  const cleaned = stem.replace(/[^A-Za-z0-9_-]+/g, "-").replace(/^[-_]+|[-_]+$/g, "");
  const base = cleaned || "imported-files";
  return (/^[A-Za-z_]/.test(base) ? base : `import-${base}`).slice(0, 64);
}

/** Add folders or files to a collection. `origin` is the data source it
 * was opened from: File Import adds from this computer or a storage; RustFS
 * adds a folder of a bucket. A folder from a storage is viewed where it is,
 * as a collection of its own or as a folder of an existing one. With none
 * (Add Files in Collections) every place that can be viewed is offered. */
export function AddFiles({
  target,
  origin,
  title,
  onClose,
  onAdded,
}: {
  target?: { space: string; folder?: string; label: string };
  origin?: "local" | "rustfs";
  title?: string;
  onClose: () => void;
  onAdded?: (space: SpaceSummary) => void;
}) {
  const client = useClient();
  const toast = useToast();
  const adding = useAction();
  const storages = useLoad(() => client.listStorages(), []);
  const spaces = useLoad(() => client.listSpaces(), []);
  const fromBucket = origin === "rustfs";
  const [destination, setDestination] = useState(fromBucket ? NEW : "");
  const [mode, setMode] = useState<"reference" | "copy">("reference");
  const [source, setSource] = useState<"files" | "folder">("files");
  const [place, setPlace] = useState(LOCAL);
  const [name, setName] = useState("");
  const [nameEdited, setNameEdited] = useState(false);
  const [files, setFiles] = useState<File[]>([]);
  const [originals, setOriginals] = useState<string[]>([]);
  const [path, setPath] = useState<string>();
  const [within, setWithin] = useState("");
  const [browsing, setBrowsing] = useState<"files" | "folder" | "place">();
  // The collection name typed over the one made from the folder's.
  const [folderName, setFolderName] = useState<string>();
  const makingNew = !target && destination === NEW;
  const total = files.reduce((sum, file) => sum + file.size, 0);
  const duplicateNames = new Set(files.map((file) => file.name)).size !== files.length;
  // A whole folder joins a collection as files do.
  const whole = source === "folder" && !target;
  // A folder, rather than files, is added: it may come from any place and
  // join any file collection, on this computer or a storage.
  const folderJoins = mode === "reference" && (fromBucket || whole);
  const collections = (spaces.data ?? []).filter((space) =>
    space.kind === "remote"
      ? false
      : space.inPlace
        ? !space.inPlace.place || folderJoins
        : space.kind === "analytical" && !fromBucket,
  );
  const places = (storages.data ?? []).filter(
    (storage) =>
      storage.viewsInPlace &&
      storage.name !== null &&
      (origin === undefined || (storage.type === "s3") === fromBucket),
  );
  // A folder for a new or a file collection may come from a storage; from
  // a bucket, the first RustFS is chosen until another is.
  const chosen = collections.find((space) => space.name === destination);
  const choosesPlace = mode === "reference" && (makingNew || fromBucket || (whole && Boolean(chosen?.inPlace)));
  const currentPlace = !choosesPlace ? LOCAL : fromBucket ? (place || (places[0]?.name ?? LOCAL)) : place;
  const remote = places.find((storage) => storage.name === currentPlace);
  const noBucket = fromBucket && storages.data !== undefined && places.length === 0;

  // A collection that no longer takes what is being added is unchosen.
  useEffect(() => {
    if (spaces.data && destination !== "" && destination !== NEW && !collections.some((space) => space.name === destination)) {
      setDestination("");
    }
  }, [spaces.data, destination, collections]);

  useEffect(() => {
    if (makingNew && !nameEdited && files[0]) setName(suggestedName(files[0]));
  }, [makingNew, nameEdited, files]);

  // A folder that becomes a collection: the name it gets, free of the names
  // taken, which the owner may change; and the collection that views this
  // very folder already, if one does.
  const becoming =
    mode === "reference" && makingNew
      ? currentPlace !== LOCAL
        ? { place: currentPlace, path: within.trim().replace(/^\/+|\/+$/g, "") }
        : whole && path
          ? { place: LOCAL, path }
          : undefined
      : undefined;
  const taken = new Set((spaces.data ?? []).map((space) => space.name));
  const base = nameFromFolder(becoming?.path.split(/[\\/]/).filter(Boolean).pop() ?? remote?.label ?? "");
  const free = [base, ...Array.from({ length: 98 }, (_, i) => `${base}-${i + 2}`)].find((x) => !taken.has(x)) ?? base;
  const asName = folderName ?? free;
  const already =
    becoming &&
    (spaces.data ?? []).find(
      (space) =>
        space.inPlace &&
        (space.inPlace.place ?? LOCAL) === becoming.place &&
        space.inPlace.path.replace(/[\\/]+$/, "") === becoming.path.replace(/[\\/]+$/, ""),
    );
  const nameOk = !becoming || (NAME.test(asName) && !taken.has(asName));

  const destinationReady = target !== undefined || destination !== "";
  const ready =
    !destinationReady || duplicateNames || !nameOk
      ? false
      : mode === "copy"
        ? (!makingNew || NAME.test(name)) && files.length > 0 && total <= MAX_IMPORT_BYTES
        : currentPlace !== LOCAL
          ? true
          : fromBucket
            ? false
            : whole
              ? path !== undefined
              : originals.length > 0;

  const add = async () => {
    const added = await adding.run(async () => {
      if (mode === "copy") {
        return client.importFiles({
          ...(makingNew ? { name } : { space: target?.space ?? destination, folder: target?.folder }),
          files: await Promise.all(files.map(async (file) => ({ name: file.name, dataBase64: await base64(file) }))),
        });
      }
      if (!makingNew) {
        if (currentPlace !== LOCAL) {
          return client.addInPlaceFolder({ space: destination, path: within.trim(), place: currentPlace });
        }
        return whole
          ? client.addInPlaceFolder({ space: destination, path: path! })
          : client.addInPlaceFiles({ space: target?.space ?? destination, files: originals });
      }
      if (currentPlace !== LOCAL) return client.viewInPlace({ place: currentPlace, path: within.trim(), name: asName });
      if (source === "folder") return client.viewInPlace({ path: path!, name: asName });
      return client.viewInPlace({ path: "", files: originals });
    }, rawMessage);
    if (!added) return;
    forgetPreviews();
    toast(mode === "copy" && makingNew ? s.imported(added.name) : makingNew ? s.viewed(added.name) : s.added(added.name));
    onAdded?.(added);
    onClose();
    if (!target || mode === "reference") navigate(`/collections/${encodeURIComponent(added.name)}`);
  };

  return (
    <>
      <Dialog
        title={title ?? s.addFiles}
        closeLabel={strings.common.close}
        onClose={onClose}
        footer={
          <>
            <Button onClick={onClose}>{strings.common.cancel}</Button>
            <Button variant="primary" disabled={!ready || adding.busy} onClick={() => void add()}>
              {adding.busy ? s.adding : s.addAction}
            </Button>
          </>
        }
      >
        <p className="oik-dialog__text">{fromBucket ? s.rustfsBody : target ? s.addFilesCurrent : s.addFilesBody}</p>
        {noBucket && (
          <p className="screen-help">
            {s.noRustfs} <Link to="/settings">{strings.nav.settings}</Link>
          </p>
        )}
        {target ? (
          <p className="screen-path" translate="no">
            {target.label}
          </p>
        ) : (
          <Select
            label={s.addTo}
            value={destination}
            onChange={(value) => {
              setDestination(value);
              adding.clear();
            }}
            options={[
              { value: "", label: s.chooseCollection },
              ...collections.map((space) => ({ value: space.name, label: space.name })),
              { value: NEW, label: s.newCollection },
            ]}
          />
        )}
        {!fromBucket && (
        <ChoiceList
          label={s.addHow}
          name="add-files-mode"
          value={mode}
          onChange={(value) => {
            setMode(value as "reference" | "copy");
            adding.clear();
          }}
          choices={[
            { value: "reference", title: s.keepInPlace, meta: s.keepInPlaceBody, leading: <Icon name="folder" /> },
            { value: "copy", title: s.copyToOikome, meta: s.copyToOikomeBody, leading: <Icon name="download" /> },
          ]}
        />
        )}
        {choosesPlace && places.length > (fromBucket ? 1 : 0) && (
          <Select
            label={s.viewOn}
            value={currentPlace}
            onChange={(value) => {
              setPlace(value);
              adding.clear();
            }}
            options={[
              ...(fromBucket ? [] : [{ value: LOCAL, label: s.thisComputerPlace }]),
              ...places.map((storage) => ({ value: storage.name!, label: storage.label })),
            ]}
          />
        )}
        {currentPlace !== LOCAL ? (
          <div className="oik-field">
            <span className="oik-field__label">{s.folderOn(remote?.label ?? currentPlace)}</span>
            <div className="screen-source-pick">
              <span className="screen-path" translate="no" title={within}>
                {within || s.wholePlace}
              </span>
              <Button onClick={() => setBrowsing("place")}>{s.chooseFolder}</Button>
            </div>
            {!makingNew && destination !== "" && (
              <p className="screen-help">{s.joinsAsFolder(destination, remote?.label ?? currentPlace)}</p>
            )}
          </div>
        ) : fromBucket ? null : mode === "reference" ? (
          <>
            {!target && (
              <Select
                label={s.addWhat}
                value={source}
                onChange={(value) => {
                  setSource(value as "files" | "folder");
                  // A folder becomes a collection of its own unless another is chosen.
                  if (value === "folder" && destination === "") setDestination(NEW);
                  adding.clear();
                }}
                options={[
                  { value: "files", label: s.selectedFiles },
                  { value: "folder", label: s.wholeFolder },
                ]}
              />
            )}
            {!whole && (
              <>
                <div className="screen-source-pick">
                  <span className="screen-path">{originals.length === 0 ? s.noFilesYet : s.selectedPaths(originals.length)}</span>
                  <Button onClick={() => setBrowsing("files")}>{s.chooseFiles}</Button>
                </div>
                {originals.length > 0 && (
                  <ul className="screen-import-files" aria-label={s.selectedFiles}>
                    {originals.map((original) => (
                      <li key={original}>
                        <span translate="no" title={original}>
                          {homeRelative(original)}
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
              </>
            )}
            {whole && (
              <div className="screen-source-pick">
                <span className="screen-path" translate="no" title={path}>
                  {path ? homeRelative(path) : s.noFolderYet}
                </span>
                <Button onClick={() => setBrowsing("folder")}>{s.chooseFolder}</Button>
              </div>
            )}
          </>
        ) : (
          <>
            <div className="screen-gapp__file">
              <label className="oik-button oik-button--secondary screen-file-pick">
                <Icon name="upload" />
                {s.chooseFiles}
                <input
                  aria-label={s.files}
                  type="file"
                  multiple
                  onChange={(event) => {
                    setFiles([...(event.target.files ?? [])]);
                    adding.clear();
                  }}
                />
              </label>
              <span className="screen-help">{files.length === 0 ? s.noFilesYet : s.importLimit}</span>
            </div>
            {files.length > 0 && (
              <ul className="screen-import-files" aria-label={s.selectedFiles}>
                {files.map((file, index) => (
                  <li key={`${file.name}:${index}`}>
                    <span translate="no">{file.name}</span>
                    <span className="screen-muted">{bytes(file.size)}</span>
                  </li>
                ))}
              </ul>
            )}
            {makingNew && (
              <TextField
                name="import-collection-name"
                label={s.collectionNameLabel}
                hint={s.importNameHint}
                placeholder={s.namePlaceholder}
                value={name}
                mono
                onChange={(value) => {
                  setName(value);
                  setNameEdited(true);
                  adding.clear();
                }}
              />
            )}
            {duplicateNames && <ErrorText>{s.duplicateFiles}</ErrorText>}
            {total > MAX_IMPORT_BYTES && <ErrorText>{s.importLimit}</ErrorText>}
          </>
        )}
        {becoming && (
          <>
            <TextField
              name="folder-collection-name"
              label={s.collectionNameLabel}
              hint={s.importNameHint}
              value={asName}
              mono
              onChange={(value) => {
                setFolderName(value);
                adding.clear();
              }}
            />
            {taken.has(asName) && <ErrorText>{s.nameTaken(asName)}</ErrorText>}
            {already && <Banner title={s.folderAlready(already.name)} />}
            {!already && folderName === undefined && free !== base && (
              <p className="screen-help">{s.nameNumbered(base, free)}</p>
            )}
          </>
        )}
        {!destinationReady && (path !== undefined || originals.length > 0 || files.length > 0) && (
          <ErrorText>{s.chooseCollection}</ErrorText>
        )}
        {adding.busy && (
          <span className="screen-progress" role="progressbar" aria-label={s.adding}>
            <i />
          </span>
        )}
        {adding.error && <ErrorText>{adding.error}</ErrorText>}
      </Dialog>
      {browsing === "files" && (
        <FolderBrowser
          title={s.chooseFiles.replace("…", "")}
          hint={() => s.referenceFilesHint}
          onClose={() => setBrowsing(undefined)}
          onChooseFiles={(paths) => {
            setOriginals(paths);
            setBrowsing(undefined);
            adding.clear();
          }}
        />
      )}
      {browsing === "place" && remote && (
        <FolderBrowser
          title={s.chooseFolder.replace("…", "")}
          place={{ name: remote.name!, label: remote.label }}
          hint={(here) => s.holdsFiles(here.files.length, here.folders.length)}
          onClose={() => setBrowsing(undefined)}
          onChoose={(picked) => {
            setWithin(picked);
            setBrowsing(undefined);
            setFolderName(undefined);
            adding.clear();
          }}
        />
      )}
      {browsing === "folder" && (
        <FolderBrowser
          title={s.chooseFolder.replace("…", "")}
          hint={(here) => s.holdsFiles(here.files.length, here.folders.length)}
          onClose={() => setBrowsing(undefined)}
          onChoose={(picked) => {
            setPath(picked);
            setBrowsing(undefined);
            setFolderName(undefined);
            adding.clear();
          }}
        />
      )}
    </>
  );
}
