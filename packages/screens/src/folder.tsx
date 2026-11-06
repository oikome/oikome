// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// View a folder where it is (3.9): a folder on this computer (a disk, a
// mounted share, a cloud drive's sync folder) chosen by browsing, or one on
// a NAS by its path there. Oikome reads it in place and never changes it;
// the collection it makes keeps only an index.

import { useState } from "react";
import { Button, Dialog, ErrorText, Select, TextField, useToast } from "@oikome/ui";
import type { SpaceSummary } from "@oikome/client";
import { forgetPreviews } from "./content";
import { useAction, useClient, useLoad } from "./data";
import { homeRelative } from "./format";
import { navigate } from "./router";
import { FolderBrowser } from "./storage";
import { strings } from "./strings";

const s = strings.collections;
const LOCAL = "";

export function ViewFolder({
  onClose,
  onViewed,
}: {
  onClose: () => void;
  onViewed?: (space: SpaceSummary) => void;
}) {
  const client = useClient();
  const toast = useToast();
  const storages = useLoad(() => client.listStorages(), []);
  // A NAS over WebDAV is named by its path there; anything else on this
  // computer (a mounted share, a drive's folder) is browsed.
  const nases = (storages.data ?? []).filter((x) => x.type === "webdav" && x.name !== null);
  const [place, setPlace] = useState(LOCAL);
  const [path, setPath] = useState<string>();
  const [within, setWithin] = useState("");
  const [browsing, setBrowsing] = useState(false);
  const view = useAction();
  const nas = nases.find((x) => x.name === place);
  const ready = place === LOCAL ? !!path : true;
  return (
    <>
      <Dialog
        title={s.viewFolder}
        closeLabel={strings.common.close}
        onClose={onClose}
        footer={
          <>
            <Button onClick={onClose}>{strings.common.cancel}</Button>
            <Button
              variant="primary"
              disabled={!ready || view.busy}
              onClick={async () => {
                const viewed = await view.run(() =>
                  client.viewInPlace(place === LOCAL ? { path: path! } : { place, path: within.trim() }),
                );
                if (!viewed) return;
                forgetPreviews();
                toast(s.viewed(viewed.name));
                onViewed?.(viewed);
                navigate(`/collections/${encodeURIComponent(viewed.name)}`);
              }}
            >
              {view.busy ? s.reading : s.viewIt}
            </Button>
          </>
        }
      >
        <p className="oik-dialog__text">{s.viewFolderBody}</p>
        {nases.length > 0 && (
          <Select
            label={s.viewOn}
            value={place}
            onChange={(value) => {
              setPlace(value);
              view.clear();
            }}
            options={[
              { value: LOCAL, label: s.thisComputerPlace },
              ...nases.map((x) => ({ value: x.name!, label: x.label })),
            ]}
          />
        )}
        {place === LOCAL ? (
          <div className="screen-source-pick">
            <span className="screen-path" translate="no" title={path}>
              {path ? homeRelative(path) : s.noFolderYet}
            </span>
            <Button onClick={() => setBrowsing(true)}>{s.chooseFolder}</Button>
          </div>
        ) : (
          <TextField
            name="folder-on-place"
            label={s.folderOn(nas?.label ?? place)}
            hint={s.folderOnHint}
            value={within}
            onChange={setWithin}
          />
        )}
        {view.busy && (
          <span className="screen-progress" role="progressbar" aria-label={s.reading}>
            <i />
          </span>
        )}
        {view.error && <ErrorText>{view.error}</ErrorText>}
      </Dialog>
      {browsing && (
        <FolderBrowser
          title={s.chooseFolder.replace("…", "")}
          hint={(here) => s.holds(here.photos, here.notes)}
          onClose={() => setBrowsing(false)}
          onChoose={(picked) => {
            setPath(picked);
            setBrowsing(false);
            view.clear();
          }}
        />
      )}
    </>
  );
}
