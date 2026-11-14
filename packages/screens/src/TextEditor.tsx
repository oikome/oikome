// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { useEffect, useState } from "react";
import { Button, Checkbox, Dialog, ErrorText, Select, TextField } from "@oikome/ui";
import { OikomeError } from "@oikome/client";
import type { ConfirmationScope, FileItem, SaveTextFile, TextSaveTarget } from "@oikome/client";
import { asSentence, errorMessage, useClient } from "./data";
import { strings } from "./strings";
import SourceView from "./SourceView";

const s = strings.editing;

function suggestedCopyName(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? `${name.slice(0, dot)} copy${name.slice(dot)}` : `${name} copy`;
}

export default function TextEditor({
  space,
  file,
  onSaved,
  onCancel,
  onReload,
  onDirtyChange,
}: {
  space: string;
  file: FileItem;
  onSaved: (file: FileItem, copy: boolean) => void;
  onCancel: () => void;
  onReload: () => void;
  onDirtyChange: (dirty: boolean) => void;
}) {
  const client = useClient();
  const [draft, setDraft] = useState(file.text ?? "");
  const [copyOpen, setCopyOpen] = useState(false);
  const [copyName, setCopyName] = useState(() => suggestedCopyName(file.name));
  const [confirmation, setConfirmation] = useState<{
    action: "source" | "overwrite";
    targetVersion?: string;
  }>();
  const [remember, setRemember] = useState(false);
  const [scope, setScope] = useState<ConfirmationScope>("directory");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [conflict, setConflict] = useState(false);
  const original = file.text ?? "";
  const dirty = draft !== original;

  useEffect(() => onDirtyChange(dirty), [dirty, onDirtyChange]);

  const save = async (
    target: TextSaveTarget,
    options: { confirm?: boolean; targetVersion?: string; remember?: ConfirmationScope } = {},
  ) => {
    setBusy(true);
    setError(undefined);
    setConflict(false);
    const request: SaveTextFile = {
      space,
      path: file.path,
      text: draft,
      version: file.content.version,
      target,
      copyName: target === "copy" ? copyName.trim() : undefined,
      confirm: options.confirm ?? false,
      targetVersion: options.targetVersion,
      remember: options.remember,
    };
    try {
      const saved = await client.saveTextFile(request);
      setCopyOpen(false);
      setConfirmation(undefined);
      onSaved(saved, target === "copy");
    } catch (e) {
      if (e instanceof OikomeError && e.reason === "source_confirmation_required") {
        setCopyOpen(false);
        setConfirmation({ action: "source" });
      } else if (e instanceof OikomeError && e.reason === "overwrite_confirmation_required") {
        setCopyOpen(false);
        setConfirmation({ action: "overwrite", targetVersion: e.params.targetVersion });
      } else {
        const changed = e instanceof OikomeError && e.reason === "version_conflict";
        setConflict(changed);
        setError(changed ? s.conflict : errorMessage(e, asSentence));
      }
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== "s") return;
      e.preventDefault();
      e.stopPropagation();
      if (file.content.capabilities.edit) void save("original");
      else setCopyOpen(true);
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  });

  return (
    <div className="screen-text-editor">
      <div className="screen-fileview__viewbar">
        {file.content.capabilities.edit && (
          <Button variant="primary" disabled={busy || !dirty} onClick={() => void save("original")}>
            {s.saveOriginal}
          </Button>
        )}
        {file.content.capabilities.saveCopy && (
          <Button variant={file.content.capabilities.edit ? "quiet" : "primary"} disabled={busy} onClick={() => setCopyOpen(true)}>
            {s.saveCopy}
          </Button>
        )}
        <Button variant="quiet" disabled={busy} onClick={onCancel}>
          {strings.common.cancel}
        </Button>
      </div>
      {!file.content.capabilities.edit && (
        <p className="screen-footnote">{file.content.capabilities.saveCopy ? s.readOnly : s.unavailable}</p>
      )}
      {error && (
        <div className="screen-text-editor__error">
          <ErrorText>{error}</ErrorText>
          {conflict && <Button variant="quiet" onClick={onReload}>{s.reload}</Button>}
        </div>
      )}
      <SourceView text={draft} name={file.name} onChange={setDraft} />

      {copyOpen && (
        <Dialog
          title={s.copyTitle}
          closeLabel={strings.common.close}
          onClose={() => setCopyOpen(false)}
          footer={
            <>
              <Button onClick={() => setCopyOpen(false)}>{strings.common.cancel}</Button>
              <Button variant="primary" disabled={busy || !copyName.trim()} onClick={() => void save("copy")}>
                {s.createCopy}
              </Button>
            </>
          }
        >
          <TextField label={s.copyName} name="copy-name" value={copyName} onChange={setCopyName} mono autoFocus selectOnFocus />
          <p className="oik-dialog__text">{s.copyHint}</p>
          {error && !conflict && <ErrorText>{error}</ErrorText>}
        </Dialog>
      )}

      {confirmation && (
        <Dialog
          title={confirmation.action === "source" ? s.changeTitle : s.overwriteTitle(copyName)}
          closeLabel={strings.common.close}
          onClose={() => setConfirmation(undefined)}
          closeButton={false}
          footer={
            <>
              <Button autoFocus onClick={() => setConfirmation(undefined)}>{strings.common.cancel}</Button>
              <Button
                variant="destructive"
                disabled={busy}
                onClick={() =>
                  void save(confirmation.action === "source" ? "original" : "copy", {
                    confirm: true,
                    targetVersion: confirmation.targetVersion,
                    remember: remember ? scope : undefined,
                  })
                }
              >
                {confirmation.action === "source" ? s.changeConfirm : s.overwriteConfirm}
              </Button>
            </>
          }
        >
          <p className="oik-dialog__text">
            {confirmation.action === "source" ? s.changeBody(file.name) : s.overwriteBody(copyName)}
          </p>
          <Checkbox label={s.remember} checked={remember} onChange={setRemember} />
          {remember && (
            <Select
              label={s.scope}
              value={scope}
              onChange={(value) => setScope(value as ConfirmationScope)}
              options={[
                { value: "identity", label: s.identity },
                { value: "directory", label: s.directory },
              ]}
            />
          )}
        </Dialog>
      )}
    </div>
  );
}
