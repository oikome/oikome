// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { useState } from "react";
import { Button, ChoiceList, Dialog, ErrorText, Icon, Select, TextField, useToast } from "@oikome/ui";
import type { SpaceSummary } from "@oikome/client";
import { forgetPreviews } from "./content";
import { rawMessage, useAction, useClient, useLoad } from "./data";
import { bytes, homeRelative } from "./format";
import { navigate } from "./router";
import { FolderBrowser } from "./storage";
import { strings } from "./strings";

const s = strings.collections;
const NAME = /^[A-Za-z_-][A-Za-z0-9_-]{0,63}$/;
const LOCAL = "";
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

export function AddFiles({
  target,
  onClose,
  onAdded,
}: {
  target?: { space: string; folder?: string; label: string };
  onClose: () => void;
  onAdded?: (space: SpaceSummary) => void;
}) {
  const client = useClient();
  const toast = useToast();
  const adding = useAction();
  const storages = useLoad(() => client.listStorages(), []);
  const [mode, setMode] = useState<"reference" | "copy">("reference");
  const [source, setSource] = useState<"files" | "folder">("files");
  const [place, setPlace] = useState(LOCAL);
  const [name, setName] = useState("");
  const [files, setFiles] = useState<File[]>([]);
  const [originals, setOriginals] = useState<string[]>([]);
  const [path, setPath] = useState<string>();
  const [within, setWithin] = useState("");
  const [browsing, setBrowsing] = useState<"files" | "folder">();
  const total = files.reduce((sum, file) => sum + file.size, 0);
  const places = (storages.data ?? []).filter((storage) => storage.viewsInPlace && storage.name !== null);
  const currentPlace = target ? LOCAL : place;
  const remote = places.find((storage) => storage.name === currentPlace);
  const ready =
    mode === "copy"
      ? (target !== undefined || NAME.test(name)) && files.length > 0 && total <= MAX_IMPORT_BYTES
      : currentPlace !== LOCAL
        ? true
        : target || source === "files"
          ? originals.length > 0
          : path !== undefined;

  const add = async () => {
    const added = await adding.run(async () => {
      if (mode === "copy") {
        return client.importFiles({
          ...(target ? { space: target.space, folder: target.folder } : { name }),
          files: await Promise.all(files.map(async (file) => ({ name: file.name, dataBase64: await base64(file) }))),
        });
      }
      if (target) return client.addInPlaceFiles({ space: target.space, files: originals });
      if (currentPlace !== LOCAL) return client.viewInPlace({ place: currentPlace, path: within.trim() });
      if (source === "folder") return client.viewInPlace({ path: path! });
      return client.viewInPlace({ path: "", files: originals });
    }, rawMessage);
    if (!added) return;
    forgetPreviews();
    toast(mode === "copy" ? s.imported(added.name) : target ? s.added(added.name) : s.viewed(added.name));
    onAdded?.(added);
    onClose();
    if (!target || mode === "reference") navigate(`/collections/${encodeURIComponent(added.name)}`);
  };

  return (
    <>
      <Dialog
        title={s.addFiles}
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
        <p className="oik-dialog__text">{target ? s.addFilesCurrent : s.addFilesBody}</p>
        {target && (
          <p className="screen-path" translate="no">
            {target.label}
          </p>
        )}
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
        {mode === "reference" && !target && places.length > 0 && (
          <Select
            label={s.viewOn}
            value={place}
            onChange={(value) => {
              setPlace(value);
              adding.clear();
            }}
            options={[
              { value: LOCAL, label: s.thisComputerPlace },
              ...places.map((storage) => ({ value: storage.name!, label: storage.label })),
            ]}
          />
        )}
        {mode === "reference" && currentPlace !== LOCAL ? (
          <TextField
            name="folder-on-place"
            label={s.folderOn(remote?.label ?? currentPlace)}
            hint={s.folderOnHint}
            value={within}
            onChange={setWithin}
          />
        ) : mode === "reference" ? (
          <>
            {!target && (
              <Select
                label={s.addWhat}
                value={source}
                onChange={(value) => {
                  setSource(value as "files" | "folder");
                  adding.clear();
                }}
                options={[
                  { value: "files", label: s.selectedFiles },
                  { value: "folder", label: s.wholeFolder },
                ]}
              />
            )}
            {(target || source === "files") && (
              <>
                <div className="screen-source-pick">
                  <span className="screen-path">{originals.length === 0 ? s.noFilesYet : s.selectedPaths(originals.length)}</span>
                  <Button onClick={() => setBrowsing("files")}>{s.chooseFiles}</Button>
                </div>
                {originals.length > 0 && (
                  <ul className="screen-import-files" aria-label={s.files}>
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
            {!target && source === "folder" && (
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
            {!target && (
              <TextField
                name="import-collection-name"
                label={s.nameLabel}
                hint={s.nameHint}
                placeholder={s.namePlaceholder}
                value={name}
                mono
                onChange={(value) => {
                  setName(value);
                  adding.clear();
                }}
              />
            )}
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
              <ul className="screen-import-files" aria-label={s.files}>
                {files.map((file, index) => (
                  <li key={`${file.name}:${index}`}>
                    <span translate="no">{file.name}</span>
                    <span className="screen-muted">{bytes(file.size)}</span>
                  </li>
                ))}
              </ul>
            )}
            {total > MAX_IMPORT_BYTES && <ErrorText>{s.importLimit}</ErrorText>}
          </>
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
      {browsing === "folder" && (
        <FolderBrowser
          title={s.chooseFolder.replace("…", "")}
          hint={(here) => s.holds(here.photos, here.notes)}
          onClose={() => setBrowsing(undefined)}
          onChoose={(picked) => {
            setPath(picked);
            setBrowsing(undefined);
            adding.clear();
          }}
        />
      )}
    </>
  );
}
