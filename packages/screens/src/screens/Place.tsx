// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// A place collections are kept, opened, as the prototype's place page:
// what it is and how full, the collections kept there, how Oikome reaches
// it, and what is lost if it breaks.

import { useState } from "react";
import { Button, Card, ConfirmDialog, EmptyState, ErrorText, Status, useToast } from "@oikome/ui";
import type { StorageSummary } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { homeRelative, listNames, usedOfTotal } from "../format";
import { ScreenHeader } from "../parts";
import { Link, navigate } from "../router";
import { strings } from "../strings";

const s = strings.place;

/** This computer's address among the places: storage names can't hold a `-`. */
export const THIS_COMPUTER = "this-computer";

/** The address of a place's page. */
export function placePath(place: StorageSummary): string {
  return `/places/${encodeURIComponent(place.name ?? THIS_COMPUTER)}`;
}

function labelOf(place: StorageSummary): string {
  return place.name === null ? strings.storage.thisComputer : place.label;
}

/** A line of the page's lists: what, then its value. */
function Fact({ term, children }: { term: string; children: React.ReactNode }) {
  return (
    <div className="screen-place-fact">
      <dt>{term}</dt>
      <dd>{children}</dd>
    </div>
  );
}

/** How Oikome reaches the place, by its kind. */
function Reached({ place, googleApp }: { place: StorageSummary; googleApp?: string | null }) {
  const where = (
    <span translate="no" title={place.place}>
      {homeRelative(place.place)}
    </span>
  );
  switch (place.type) {
    case "disk":
      return <Fact term={s.writesIn}>{where}</Fact>;
    case "folder":
    case "drive":
      return <Fact term={s.usesOnly}>{where}</Fact>;
    case "webdav":
      return (
        <>
          <Fact term={s.address}>{where}</Fact>
          <Fact term={s.password}>{s.inKeychain}</Fact>
        </>
      );
    case "s3":
      return (
        <>
          <Fact term={s.bucketAt}>{where}</Fact>
          <Fact term={s.secretKey}>{s.inKeychain}</Fact>
        </>
      );
    case "gdrive":
      return (
        <>
          <Fact term={s.usesOnly}>{s.googleFolder}</Fact>
          {googleApp && (
            <Fact term={s.googleApp}>
              <span translate="no">{googleApp}</span>
            </Fact>
          )}
          <Fact term={s.signIn}>{s.inKeychain}</Fact>
          <Fact term={s.otherFiles}>{place.viewsInPlace ? s.otherSeen : s.otherNotSeen}</Fact>
        </>
      );
  }
}

export function PlaceDetail({ name }: { name: string }) {
  const client = useClient();
  const toast = useToast();
  const storages = useLoad(() => client.listStorages(), []);
  const spaces = useLoad(() => client.listSpaces(), []);
  const googleApp = useLoad(() => client.googleApp(), []);
  const change = useAction();
  const remove = useAction();
  const [removing, setRemoving] = useState(false);

  if (storages.error) return <ErrorText>{storages.error}</ErrorText>;
  if (!storages.data) return <div className="screen-page-skeleton oik-skel" aria-hidden="true" />;
  const place = storages.data.find((x) => (x.name ?? THIS_COMPUTER) === name);
  if (!place) {
    return (
      <Card>
        <EmptyState title={s.goneTitle} body={s.goneBody} action={<Link to="/settings">{strings.nav.settings}</Link>} />
      </Card>
    );
  }
  const label = labelOf(place);
  // Collections of one's own kept here; those shared with you live elsewhere.
  const here = (spaces.data ?? []).filter((x) => x.storage === place.name && !x.fromDid && !x.inPlace);
  const known = place.totalBytes != null && place.freeBytes != null;
  const kind = place.name === null ? strings.storage.thisComputer : strings.settings.storageKind[place.type];

  return (
    <>
      <ScreenHeader label={s.label} title={label} lede={kind}>
        {!place.isDefault && (
          <Button
            variant="primary"
            disabled={change.busy}
            onClick={async () => {
              const done = await change.run(() => client.setDefaultStorage({ storage: place.name }));
              if (done) {
                toast(strings.toast.defaultStorage(label));
                storages.reload();
              }
            }}
          >
            {strings.storage.useForNew}
          </Button>
        )}
        {place.name !== null && (
          <Button variant="quiet" onClick={() => setRemoving(true)}>
            {strings.storage.remove}
          </Button>
        )}
      </ScreenHeader>
      {change.error && <ErrorText>{change.error}</ErrorText>}

      <dl className="oik-card screen-facts">
        <div>
          <dt className="oik-label">{s.room}</dt>
          <dd>
            {known
              ? usedOfTotal(place.totalBytes! - place.freeBytes!, place.totalBytes!)
              : s.roomUnknown}
          </dd>
        </div>
        <div>
          <dt className="oik-label">{s.encryption}</dt>
          <dd>
            <Status kind={place.encrypted ? "ok" : "idle"}>{place.encrypted ? s.encrypted : s.notEncrypted}</Status>
          </dd>
        </div>
        <div>
          <dt className="oik-label">{s.newCollections}</dt>
          <dd>{place.isDefault ? s.goHere : s.goElsewhere}</dd>
        </div>
      </dl>

      <div className="screen-dash screen-dash--split">
        <section className="oik-card screen-panel" aria-label={s.keptHere}>
          <header className="screen-panel__head">
            <h2 className="oik-label">{s.keptHere}</h2>
            <span className="screen-panel__aside">{strings.settings.storageCount(here.length)}</span>
          </header>
          {spaces.error ? (
            <p className="screen-panel__empty">{spaces.error}</p>
          ) : here.length === 0 ? (
            <p className="screen-panel__empty">{s.nothingHere}</p>
          ) : (
            <ul className="screen-place-list">
              {here.map((space) => (
                <li key={space.name}>
                  <Link to={`/collections/${encodeURIComponent(space.name)}`} className="oik-link screen-link-mono">
                    {space.name}
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="oik-card screen-panel" aria-label={s.reached}>
          <header className="screen-panel__head">
            <h2 className="oik-label">{s.reached}</h2>
          </header>
          <dl className="screen-place-facts">
            <Reached place={place} googleApp={googleApp.data?.clientId} />
          </dl>
        </section>

        <section className="oik-card screen-panel" aria-label={s.breaks}>
          <header className="screen-panel__head">
            <h2 className="oik-label">{s.breaks}</h2>
          </header>
          <p className="screen-place-text">
            {here.length === 0
              ? s.breaksNothing
              : place.name === null
                ? s.breaksLocal(listNames(here.map((x) => x.name)), here.length)
                : s.breaksPlace(label, listNames(here.map((x) => x.name)), here.length)}
          </p>
          {place.encrypted && <p className="screen-place-text">{s.breaksKey}</p>}
        </section>
      </div>

      {removing && (
        <ConfirmDialog
          title={strings.storage.removeTitle(label)}
          body={
            here.length > 0
              ? strings.storage.stillHoldsBody(
                  here.map((x) => x.name),
                  here.length,
                )
              : place.type === "dropbox"
                ? strings.storage.removeBodyDropbox
                : place.type === "gdrive"
                  ? strings.storage.removeBodyGdrive
                  : place.encrypted
                  ? strings.storage.removeBodyEncrypted
                  : strings.storage.removeBody
          }
          confirmLabel={strings.storage.removeYes}
          cancelLabel={strings.common.cancel}
          busy={remove.busy}
          error={remove.error}
          onConfirm={async () => {
            if (place.name === null) return;
            const done = await remove.run(() => client.removeStorage({ storage: place.name! }));
            if (done) {
              toast(strings.toast.storageRemoved(label));
              navigate("/settings");
            }
          }}
          onCancel={() => {
            setRemoving(false);
            remove.clear();
          }}
        />
      )}
    </>
  );
}
