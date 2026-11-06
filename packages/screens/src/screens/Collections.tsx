// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Collections: every collection as a cover, and making a new one.

import { useEffect, useState } from "react";
import {
  Button,
  Card,
  Dialog,
  EmptyState,
  ErrorText,
  Icon,
  Segmented,
  Select,
  TextField,
  collectionTones,
  useToast,
} from "@oikome/ui";
import type { IconName } from "@oikome/ui";
import type { StorageSummary } from "@oikome/client";
import { forgetPreviews, loadPreview } from "../content";
import type { View } from "../content";
import { Cover, sharedNames } from "../covers";
import { useAction, useClient, useLoad } from "../data";
import { homeRelative } from "../format";
import { ScreenHeader } from "../parts";
import { navigate } from "../router";
import { strings } from "../strings";
import { ViewFolder } from "../folder";

const s = strings.collections;
/** The node's rule for a name: ASCII letters, digits, _ and -, not led by a digit, 1 to 64. */
const NAME = /^[A-Za-z_-][A-Za-z0-9_-]{0,63}$/;

const STORAGE_ICONS: Record<StorageSummary["type"], IconName> = {
  disk: "computer",
  webdav: "nas",
  folder: "folder",
  drive: "drive",
};

/** What a storage is called: this computer, or the name the owner gave it. */
export function storageLabel(storage: StorageSummary): string {
  return storage.name === null ? s.thisComputer : storage.label;
}

export function Collections({ startCreating = false }: { startCreating?: boolean }) {
  const client = useClient();
  const spaces = useLoad(() => client.listSpaces(), []);
  const shares = useLoad(() => client.listShares(), []);
  const [creating, setCreating] = useState(startCreating);
  const [viewing, setViewing] = useState(false);
  useEffect(() => {
    if (startCreating) setCreating(true);
  }, [startCreating]);

  const [show, setShow] = useState<"all" | View>("all");
  const [kinds, setKinds] = useState<Map<string, View>>(new Map());
  const shared = sharedNames(shares.data);
  const list = spaces.data ?? [];
  // What each holds, from the same first rows its cover reads.
  useEffect(() => {
    let current = true;
    for (const space of spaces.data ?? []) {
      if (space.kind === "remote") continue;
      loadPreview(client, space)
        .then((preview) => {
          if (current) setKinds((m) => new Map(m).set(space.name, preview.shape.view));
        })
        .catch(() => undefined);
    }
    return () => {
      current = false;
    };
  }, [client, spaces.data]);
  // Readings are a table over time: Tables shows them too.
  const shown = (space: { name: string }) => {
    const kind = kinds.get(space.name);
    return show === "all" || kind === show || (show === "table" && kind === "series");
  };
  const mine = list.filter((space) => space.kind !== "remote");
  const fromOthers = list.filter((space) => space.kind === "remote");
  const toneOf = collectionTones(list.map((space) => space.name));
  const empty = spaces.data !== undefined && list.length === 0;

  const stopCreating = () => {
    setCreating(false);
    // Drop ?new=1, so a reload does not open the form again.
    if (startCreating) navigate(window.location.pathname, { replace: true });
  };

  return (
    <>
      <ScreenHeader title={s.title} lede={spaces.data && !empty ? s.lede(mine.length, fromOthers.length) : undefined}>
        {!empty && mine.length > 1 && (
          <Segmented
            label={s.filter}
            value={show}
            onChange={(value) => setShow(value as "all" | View)}
            options={(["all", "photos", "notes", "table"] as const).map((k) => ({ value: k, label: s.filters[k] }))}
          />
        )}
        {/* An empty page offers the same actions once, in its empty state. */}
        {!empty && (
          <Button icon="folder" onClick={() => setViewing(true)}>
            {s.viewFolder}
          </Button>
        )}
        {!empty && (
          <Button variant="primary" icon="plus" onClick={() => setCreating(true)}>
            {s.newCollection}
          </Button>
        )}
      </ScreenHeader>

      {creating && <NewCollection onClose={stopCreating} onCreated={spaces.reload} />}
      {viewing && <ViewFolder onClose={() => setViewing(false)} onViewed={() => spaces.reload()} />}

      {spaces.error && <ErrorText>{spaces.error}</ErrorText>}
      {!spaces.data && !spaces.error && (
        <div className="screen-covers" aria-hidden="true">
          {[0, 1, 2].map((i) => (
            <div key={i} className="screen-cover">
              <div className="screen-cover__art oik-skel" />
            </div>
          ))}
        </div>
      )}
      {empty && (
        <Card>
          <EmptyState
            icon="collection"
            title={s.emptyTitle}
            body={s.emptyBody}
            action={
              <>
                <Button icon="folder" onClick={() => setViewing(true)}>
                  {s.viewFolder}
                </Button>
                <Button variant="primary" icon="plus" onClick={() => setCreating(true)}>
                  {s.newCollection}
                </Button>
              </>
            }
          />
        </Card>
      )}

      {mine.length > 0 && (
        <section className="screen-section" aria-labelledby="mine">
          <h2 id="mine" className="oik-label screen-section__title">
            {s.yours}
          </h2>
          <div className="screen-covers">
            {mine.filter(shown).map((space) => (
              <Cover
                key={space.name}
                space={space}
                tone={toneOf(space.name)}
                names={shared.get(space.name) ?? []}
              />
            ))}
          </div>
          {mine.some((space) => space.example) && <p className="screen-footnote">{s.examplesNote}</p>}
        </section>
      )}
      {fromOthers.length > 0 && (
        <section className="screen-section" aria-labelledby="from-others">
          <h2 id="from-others" className="oik-label screen-section__title">
            {s.sharedWithYou}
          </h2>
          <div className="screen-covers">
            {fromOthers.map((space) => (
              <Cover key={space.name} space={space} tone={toneOf(space.name)} names={[]} />
            ))}
          </div>
        </section>
      )}
    </>
  );
}

/** Making a collection is a short task in a dialog: a name, and where it is kept. */
function NewCollection({ onClose, onCreated }: { onClose: () => void; onCreated: () => void }) {
  const client = useClient();
  const storages = useLoad(() => client.listStorages(), []);
  const [name, setName] = useState("");
  // "" is this computer, the default.
  const [storage, setStorage] = useState("");
  const create = useAction();
  const toast = useToast();
  const choices = storages.data ?? [];
  const here = choices.find((x) => x.name === null);
  // The default place is chosen already.
  useEffect(() => {
    const preferred = storages.data?.find((x) => x.isDefault);
    if (preferred) setStorage(preferred.name ?? "");
  }, [storages.data]);

  return (
    <Dialog
      title={s.newCollection}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button type="submit" form="new-collection" variant="primary" disabled={create.busy}>
            {s.create}
          </Button>
        </>
      }
    >
      <form
        id="new-collection"
        onSubmit={async (e) => {
          e.preventDefault();
          const wanted = name.trim();
          if (!wanted) {
            create.fail(strings.common.nameRequired);
            return;
          }
          if (!NAME.test(wanted)) {
            create.fail(s.nameInvalid);
            return;
          }
          const space = await create.run(
            () => client.createSpace({ name: wanted, storage: storage || undefined }),
            (err) => (err.code === "conflict" ? s.nameTaken(wanted) : undefined),
          );
          if (space) {
            toast(strings.toast.created(space.name));
            forgetPreviews();
            onCreated();
            onClose();
            navigate(`/collections/${encodeURIComponent(space.name)}`);
          }
        }}
      >
        <TextField
          label={s.nameLabel}
          name="collection-name"
          hint={s.nameHint}
          placeholder={s.namePlaceholder}
          value={name}
          onChange={(value) => {
            setName(value);
            create.clear();
          }}
          error={create.error}
          autoFocus
        />
        {/* The default is already chosen; a choice appears once there is one to make. */}
        {choices.length > 1 ? (
          <Select
            label={s.storeOn}
            value={storage}
            onChange={setStorage}
            leading={
              <Icon name={STORAGE_ICONS[choices.find((x) => (x.name ?? "") === storage)?.type ?? "disk"]} />
            }
            options={choices.map((x) => ({ value: x.name ?? "", label: storageLabel(x) }))}
          />
        ) : (
          <div className="oik-field">
            <span className="oik-field__label">{s.storeOn}</span>
            <div className="screen-storage-fixed">
              <Icon name="computer" />
              <span>{s.thisComputer}</span>
              {here && (
                <span className="screen-path" title={here.place} translate="no">
                  {homeRelative(here.place)}
                </span>
              )}
            </div>
            <span className="oik-field__hint">{s.storeHint}</span>
          </div>
        )}
      </form>
    </Dialog>
  );
}
