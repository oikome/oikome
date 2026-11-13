// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// A RustFS server, the object storage Oikome supports (3.4e): `rustfs` on
// this computer (`brew install rustfs/tap/rustfs`; CI installs it the same
// way), in a temporary folder, on a port of its own, with keys made up per
// run and never printed. Without it the tests that need it are skipped,
// except on CI, where that fails.

import { spawn, spawnSync } from "node:child_process";
import { createHash, createHmac, randomBytes } from "node:crypto";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

// The region requests are signed for: RustFS's own.
const REGION = "us-east-1";

/** A free port on 127.0.0.1. */
function freePort() {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

/**
 * A running RustFS: `{ endpoint, accessKey, secretKey, stop() }`, or `null`
 * where there is none (and not on CI).
 */
export async function startRustfs({ root } = {}) {
  const found = spawnSync("rustfs", ["--version"], { stdio: "ignore" }).status === 0;
  if (!found) {
    if (process.env.CI || root) throw new Error("no RustFS: install it (brew install rustfs/tap/rustfs)");
    return null;
  }
  const api = await freePort();
  const dir = root ?? mkdtempSync(join(tmpdir(), "oikome-rustfs-"));
  mkdirSync(dir, { recursive: true });
  // RustFS keeps its files in a folder that must exist already.
  const volume = join(dir, "data");
  mkdirSync(volume);
  const accessKey = `oikome${randomBytes(4).toString("hex")}`;
  const secretKey = randomBytes(16).toString("hex");
  const child = spawn("rustfs", ["server", "--address", `127.0.0.1:${api}`, volume], {
    stdio: "ignore",
    env: { ...process.env, RUSTFS_ACCESS_KEY: accessKey, RUSTFS_SECRET_KEY: secretKey },
  });
  const endpoint = `http://127.0.0.1:${api}`;
  const stop = () => {
    child.kill();
    rmSync(dir, { recursive: true, force: true });
  };
  // Ready, not only up: `/health` answers before its keys and disks serve requests.
  for (let i = 0; i < 300; i++) {
    const live = await fetch(`${endpoint}/health/ready`).then(
      (r) => r.ok,
      () => false,
    );
    if (live) return { endpoint, accessKey, secretKey, stop };
    await new Promise((r) => setTimeout(r, 100));
  }
  stop();
  throw new Error("RustFS did not start");
}

const sha256 = (data) => createHash("sha256").update(data).digest("hex");
const hmac = (key, data) => createHmac("sha256", key).update(data).digest();

/** Make the bucket `name`, as the storage's owner does, with a request signed with AWS Signature Version 4. */
export async function createBucket({ endpoint, accessKey, secretKey }, name) {
  const amzDate = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}/, "");
  const date = amzDate.slice(0, 8);
  const host = new URL(endpoint).host;
  const payload = sha256("");
  const canonical = [
    "PUT",
    `/${name}`,
    "",
    `host:${host}`,
    `x-amz-content-sha256:${payload}`,
    `x-amz-date:${amzDate}`,
    "",
    "host;x-amz-content-sha256;x-amz-date",
    payload,
  ].join("\n");
  const scope = `${date}/${REGION}/s3/aws4_request`;
  const toSign = ["AWS4-HMAC-SHA256", amzDate, scope, sha256(canonical)].join("\n");
  let key = hmac(`AWS4${secretKey}`, date);
  for (const part of [REGION, "s3", "aws4_request"]) key = hmac(key, part);
  const signature = createHmac("sha256", key).update(toSign).digest("hex");
  const response = await fetch(`${endpoint}/${name}`, {
    method: "PUT",
    headers: {
      "x-amz-date": amzDate,
      "x-amz-content-sha256": payload,
      authorization: `AWS4-HMAC-SHA256 Credential=${accessKey}/${scope}, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature=${signature}`,
    },
  });
  if (!response.ok) throw new Error(`making the bucket: ${response.status}`);
}
