// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { useState } from "react";
import { Button, Card, ErrorText, Status } from "@oikome/ui";
import type { PublicManifest, PublicSession } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { ScreenHeader } from "../parts";
import { strings } from "../strings";

const s = strings.visitor;

type Ethereum = {
  request: (request: { method: string; params?: unknown[] }) => Promise<unknown>;
};

function ethereum(): Ethereum | undefined {
  return (window as unknown as { ethereum?: Ethereum }).ethereum;
}

function base58(bytes: Uint8Array): string {
  const alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
  let value = BigInt(`0x${hex(bytes) || "0"}`);
  let out = "";
  while (value > 0) {
    out = alphabet[Number(value % 58n)] + out;
    value /= 58n;
  }
  return "1".repeat(bytes.findIndex((byte) => byte !== 0) < 0 ? bytes.length : bytes.findIndex((byte) => byte !== 0)) + out;
}

function hex(bytes: Uint8Array): string {
  return [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function textBytes(text: string): Uint8Array<ArrayBuffer> {
  const encoded = new TextEncoder().encode(text);
  return new Uint8Array(encoded.byteLength).map((_, index) => encoded[index]!);
}

export function PublicPage({ id }: { id: string }) {
  const client = useClient();
  const action = useAction();
  const manifest = useLoad(() => client.publicManifest({ id }), [id]);
  const [session, setSession] = useState<PublicSession>();
  const [paths, setPaths] = useState<string[]>([]);

  const signIn = async (wallet: boolean) => {
    const result = await action.run(async () => {
      let did: string;
      let sign: (message: string) => Promise<string>;
      if (wallet) {
        const provider = ethereum();
        if (!provider) throw new Error(s.noWallet);
        const accounts = (await provider.request({ method: "eth_requestAccounts" })) as string[];
        const chain = (await provider.request({ method: "eth_chainId" })) as string;
        const address = accounts[0];
        if (!address) throw new Error(s.noWallet);
        did = `did:pkh:eip155:${Number.parseInt(chain, 16)}:${address}`;
        sign = async (message) =>
          (await provider.request({ method: "personal_sign", params: [hex(textBytes(message)), address] })) as string;
      } else {
        const keys = (await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"])) as CryptoKeyPair;
        const publicKey = new Uint8Array(await crypto.subtle.exportKey("raw", keys.publicKey));
        did = `did:key:z${base58(new Uint8Array([0xed, 0x01, ...publicKey]))}`;
        sign = async (message) => hex(new Uint8Array(await crypto.subtle.sign("Ed25519", keys.privateKey, textBytes(message))));
      }
      const challenge = await client.publicAuthChallenge({ id, did });
      return client.publicAuthVerify({ id, did, message: challenge.message, signature: await sign(challenge.message) });
    });
    if (!result) return;
    setSession(result);
    const rows = await client.publicQuery({
      id,
      token: result.token,
      sql: "SELECT path FROM _oikome_files ORDER BY position",
    });
    setPaths(rows.table.toArray().map((row) => String((row as { path?: unknown }).path ?? "")));
  };

  const data = manifest.data as PublicManifest | undefined;
  // A photo is shown by its preview: smaller, without its metadata, and
  // there whether copying is allowed or not.
  const previewUrl = (path: string) => `/public/${id}/previews/${path.split("/").map(encodeURIComponent).join("/")}`;
  return (
    <main className="screen-public">
      <ScreenHeader label={s.publicCollection} title={data?.title ?? s.loading} lede={data ? s.byOwner(data.owner) : undefined} />
      {manifest.error && <Card><ErrorText>{manifest.error}</ErrorText></Card>}
      {data && (
        <>
          <p className="screen-help">{data.allowCopy ? s.copyBody : s.queryBody}</p>
          {data.allowCopy && (
            <a className="oik-button oik-button--secondary" href={`/public/${id}/snapshot.zip`} download>
              {s.download}
            </a>
          )}
          {!session ? (
            <Card className="screen-public__signin">
              <h2>{s.signIn}</h2>
              <p>{s.signInBody}</p>
              <div className="screen-public__actions">
                <Button variant="primary" disabled={action.busy} onClick={() => void signIn(false)}>
                  {s.temporary}
                </Button>
                <Button disabled={action.busy || !ethereum()} onClick={() => void signIn(true)}>
                  {s.wallet}
                </Button>
              </div>
              {action.error && <ErrorText>{action.error}</ErrorText>}
            </Card>
          ) : (
            <Card className="screen-public__signin">
              <Status kind="ok">{s.signedIn}</Status>
              <p className="oik-mono" translate="no">{session.did}</p>
              <p>{s.liveRows(paths.length)}</p>
            </Card>
          )}
          <section aria-label={s.files}>
            <h2>{s.files}</h2>
            {data.files.length === 0 && <p>{s.noFiles}</p>}
            <div className="screen-public__grid">
              {data.files.map((file) => (
                <Card key={file.path} className="screen-public__file">
                  {file.kind === "image" && <img src={previewUrl(file.path)} alt={file.caption ?? file.name} loading="lazy" />}
                  <h3>{file.caption ?? file.name}</h3>
                  <p className="oik-mono" translate="no">{file.path}</p>
                  {file.taken && <p>{file.taken}</p>}
                </Card>
              ))}
            </div>
          </section>
        </>
      )}
    </main>
  );
}
