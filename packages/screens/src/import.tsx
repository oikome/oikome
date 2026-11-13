// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { useState } from "react";
import { Button, Dialog, ErrorText, Icon, TextField, useToast } from "@oikome/ui";
import type { SpaceSummary } from "@oikome/client";
import { forgetPreviews } from "./content";
import { rawMessage, useAction, useClient } from "./data";
import { bytes } from "./format";
import { navigate } from "./router";
import { strings } from "./strings";

const s = strings.collections;
const NAME = /^[A-Za-z_-][A-Za-z0-9_-]{0,63}$/;
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

export function ImportFiles({
  onClose,
  onImported,
}: {
  onClose: () => void;
  onImported?: (space: SpaceSummary) => void;
}) {
  const client = useClient();
  const toast = useToast();
  const importing = useAction();
  const [name, setName] = useState("");
  const [files, setFiles] = useState<File[]>([]);
  const total = files.reduce((sum, file) => sum + file.size, 0);
  const ready = NAME.test(name) && files.length > 0 && total <= MAX_IMPORT_BYTES;

  return (
    <Dialog
      title={s.importFiles}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button
            variant="primary"
            disabled={!ready || importing.busy}
            onClick={async () => {
              const imported = await importing.run(
                async () =>
                  client.importFiles({
                    name,
                    files: await Promise.all(files.map(async (file) => ({ name: file.name, dataBase64: await base64(file) }))),
                  }),
                rawMessage,
              );
              if (!imported) return;
              forgetPreviews();
              toast(s.imported(imported.name));
              onImported?.(imported);
              navigate(`/collections/${encodeURIComponent(imported.name)}`);
            }}
          >
            {importing.busy ? s.importing : s.importAction}
          </Button>
        </>
      }
    >
      <p className="oik-dialog__text">{s.importFilesBody}</p>
      <TextField
        name="import-collection-name"
        label={s.nameLabel}
        hint={s.nameHint}
        placeholder={s.namePlaceholder}
        value={name}
        mono
        onChange={(value) => {
          setName(value);
          importing.clear();
        }}
      />
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
              importing.clear();
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
      {importing.error && <ErrorText>{importing.error}</ErrorText>}
    </Dialog>
  );
}
