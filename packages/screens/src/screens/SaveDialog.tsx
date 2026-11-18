// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { useEffect, useState } from "react";
import { Button, Checkbox, Dialog, ErrorText, Select, TextField, useToast } from "@oikome/ui";
import type { FileItem, Saved, SpaceSummary } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { navigate } from "../router";
import { strings } from "../strings";

const s = strings.save;
const NAME = /^[A-Za-z_-][A-Za-z0-9_-]{0,63}$/;

export function SaveDialog({
  space,
  onClose,
  onSaved,
}: {
  space: SpaceSummary;
  onClose: () => void;
  onSaved: (saved: Saved) => void;
}) {
  const client = useClient();
  const toast = useToast();
  const action = useAction();
  const storages = useLoad(() => client.listStorages(), []);
  const home = useLoad(() => client.listFolders(), []);
  const listing = useLoad(
    async () => (space.inPlace ? client.listFiles({ space: space.name, deep: true, limit: 1000 }) : undefined),
    [space.name],
  );
  const [files, setFiles] = useState<FileItem[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [captions, setCaptions] = useState<Record<string, string>>({});
  const [destination, setDestination] = useState<"space" | "files">("space");
  const [name, setName] = useState(`${space.name}-saved`.slice(0, 64));
  const [title, setTitle] = useState(space.name);
  const [cover, setCover] = useState("");
  const [place, setPlace] = useState("");
  const [path, setPath] = useState("");
  const [folderRule, setFolderRule] = useState("{taken:yyyy}/{taken:MM}");
  const [nameRule, setNameRule] = useState("{name}");

  const homePath = home.data?.path;
  useEffect(() => {
    if (homePath) setPath((current) => current || `${homePath}/Saved/${space.name}`);
  }, [homePath, space.name]);

  useEffect(() => {
    const next = listing.data?.files ?? [];
    setFiles(next);
    setSelected(new Set(next.map((file) => file.path)));
    setCaptions(Object.fromEntries(next.map((file) => [file.path, file.title ?? ""])));
    setCover((current) => current || next.find((file) => file.kind === "image")?.path || "");
  }, [listing.data]);

  const chosen = files.filter((file) => selected.has(file.path));
  const all = files.length > 0 && chosen.length === files.length;
  const ready = destination === "space" ? NAME.test(name) && title.trim().length > 0 : path.trim().length > 0;
  const submit = async () => {
    const saved = await action.run(() =>
      client.saveTo({
        source: space.name,
        ...(space.inPlace ? { paths: chosen.map((file) => file.path) } : {}),
        destination:
          destination === "space"
            ? {
                kind: "space",
                name,
                storage: place || null,
                title: title.trim(),
                cover: cover || null,
                captions: Object.fromEntries(chosen.map((file) => [file.path, captions[file.path]?.trim() ?? ""])),
              }
            : {
                kind: "files",
                place: place || null,
                path: path.trim(),
                folder_rule: folderRule,
                name_rule: nameRule,
              },
      }),
    );
    if (!saved) return;
    toast(destination === "space" ? s.savedCollection : s.savedFiles(saved.files.length));
    onSaved(saved);
    onClose();
    if (saved.space) navigate(`/collections/${encodeURIComponent(saved.space.name)}`);
  };

  return (
    <Dialog
      title={s.title(space.name)}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button
            variant="primary"
            disabled={!ready || action.busy || (space.inPlace !== null && chosen.length === 0)}
            onClick={() => void submit()}
          >
            {action.busy ? s.saving : s.action}
          </Button>
        </>
      }
    >
      <p className="oik-dialog__text">{s.body}</p>
      <Select
        label={s.destination}
        value={destination}
        onChange={(value) => setDestination(value as "space" | "files")}
        options={[
          { value: "space", label: s.newCollection },
          { value: "files", label: s.plainFiles, disabled: space.inPlace === null },
        ]}
      />
      <Select
        label={s.storage}
        value={place}
        onChange={setPlace}
        options={[
          { value: "", label: strings.collections.thisComputer },
          ...(storages.data ?? []).filter((storage) => storage.name !== null).map((storage) => ({
            value: storage.name!,
            label: storage.label,
          })),
        ]}
      />
      {destination === "space" ? (
        <>
          <TextField name="save-name" label={s.collectionName} value={name} mono onChange={setName} />
          <TextField name="save-title" label={s.portfolioTitle} value={title} onChange={setTitle} />
          {files.some((file) => file.kind === "image") && (
            <Select
              label={s.cover}
              value={cover}
              onChange={setCover}
              options={[
                { value: "", label: s.noCover },
                ...files.filter((file) => file.kind === "image").map((file) => ({ value: file.path, label: file.name })),
              ]}
            />
          )}
        </>
      ) : (
        <>
          <TextField name="save-path" label={s.folderPath} value={path} mono onChange={setPath} />
          <TextField name="save-folder-rule" label={s.folderRule} value={folderRule} mono onChange={setFolderRule} />
          <TextField name="save-name-rule" label={s.nameRule} value={nameRule} mono onChange={setNameRule} />
        </>
      )}
      {space.inPlace !== null && (
        <section className="screen-share-access" aria-label={s.selection}>
          <h3 className="screen-share-access__title">{s.selection}</h3>
          {files.length > 1 && (
            <div className="screen-save-all">
              <Checkbox
                label={s.selectAll}
                checked={all}
                indeterminate={chosen.length > 0 && !all}
                onChange={(checked) => setSelected(checked ? new Set(files.map((file) => file.path)) : new Set())}
              />
              <span className="screen-save-all__count">{s.chosen(chosen.length, files.length)}</span>
            </div>
          )}
          {chosen.length === 0 && <p className="screen-help">{s.noFiles}</p>}
          <ul className="screen-share-access__list">
            {files.map((file) => (
              <li key={file.path} className="screen-share-person screen-save-file">
                <Checkbox
                  label={file.path}
                  checked={selected.has(file.path)}
                  onChange={(checked) => {
                    const next = new Set(selected);
                    if (checked) next.add(file.path);
                    else next.delete(file.path);
                    setSelected(next);
                  }}
                />
                {destination === "space" && selected.has(file.path) && (
                  <TextField
                    name={`caption-${file.path}`}
                    label={s.caption(file.name)}
                    value={captions[file.path] ?? ""}
                    onChange={(value) => setCaptions({ ...captions, [file.path]: value })}
                  />
                )}
              </li>
            ))}
          </ul>
        </section>
      )}
      {listing.error && <ErrorText>{listing.error}</ErrorText>}
      {action.error && <ErrorText>{action.error}</ErrorText>}
    </Dialog>
  );
}
