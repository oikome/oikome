// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { spawn } from "node:child_process";
import { once } from "node:events";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";
import { BIN, ENV, oikome } from "./node.mjs";
import { createBucket, startRustfs } from "./rustfs.mjs";

const REPO = fileURLToPath(new URL("..", import.meta.url));
const MARKER = ".oikome-scenario.json";
const WEB_ROOT = join(REPO, "apps", "web", "dist");
const GENERATED = join(REPO, "tests", ".generated", "acceptance");

function manifest() {
  return JSON.parse(readFileSync(join(REPO, "tests", "scenarios", "acceptance", "scenario.json"), "utf8"));
}

function resetDirectory(root, firstRun) {
  if (existsSync(root)) {
    const marker = join(root, MARKER);
    if (!existsSync(marker)) throw new Error(`${root} is not an acceptance directory; refusing to replace it`);
    const saved = JSON.parse(readFileSync(marker, "utf8"));
    if (saved.scenario !== "acceptance" || saved.generatedBy !== "oikome-acceptance") {
      throw new Error(`${root} belongs to another scenario; refusing to replace it`);
    }
    rmSync(root, { recursive: true });
  }
  mkdirSync(root, { recursive: true });
  writeFileSync(join(root, MARKER), JSON.stringify({ generatedBy: "oikome-acceptance", scenario: "acceptance", firstRun }));
}

function write(root, path, contents) {
  const file = resolve(root, path);
  if (!file.startsWith(resolve(root) + "/")) throw new Error(`Invalid acceptance path ${path}`);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, contents);
}

function writeWav(root) {
  const rate = 8000;
  const samples = Buffer.alloc(rate);
  for (let i = 0; i < rate; i++) samples[i] = 128 + Math.round(Math.sin(i / 20) * 32);
  const wav = Buffer.alloc(44 + samples.length);
  wav.write("RIFF", 0);
  wav.writeUInt32LE(36 + samples.length, 4);
  wav.write("WAVEfmt ", 8);
  wav.writeUInt32LE(16, 16);
  wav.writeUInt16LE(1, 20);
  wav.writeUInt16LE(1, 22);
  wav.writeUInt32LE(rate, 24);
  wav.writeUInt32LE(rate, 28);
  wav.writeUInt16LE(1, 32);
  wav.writeUInt16LE(8, 34);
  wav.write("data", 36);
  wav.writeUInt32LE(samples.length, 40);
  samples.copy(wav, 44);
  write(root, "media/field-recording.wav", wav);
}

async function writeWebm(root) {
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage();
    const bytes = await page.evaluate(async () => {
      const canvas = document.createElement("canvas");
      canvas.width = 480;
      canvas.height = 270;
      const context = canvas.getContext("2d");
      const stream = canvas.captureStream(15);
      const chunks = [];
      const recorder = new MediaRecorder(stream, { mimeType: "video/webm" });
      recorder.ondataavailable = (event) => chunks.push(event.data);
      const done = new Promise((resolve) => {
        recorder.onstop = async () => {
          stream.getTracks().forEach((track) => track.stop());
          const bytes = new Uint8Array(await new Blob(chunks, { type: "video/webm" }).arrayBuffer());
          resolve(Array.from(bytes));
        };
      });
      recorder.start();
      let frame = 0;
      const drawing = setInterval(() => {
        context.fillStyle = frame % 2 ? "#386641" : "#bc4749";
        context.fillRect(0, 0, 480, 270);
        context.fillStyle = "white";
        context.font = "28px sans-serif";
        context.fillText(`Lisbon · frame ${frame++}`, 32, 60);
      }, 60);
      await new Promise((resolve) => setTimeout(resolve, 1200));
      clearInterval(drawing);
      recorder.stop();
      return done;
    });
    write(root, "media/tram.webm", new Uint8Array(bytes));
  } finally {
    await browser.close();
  }
}

function validPdf() {
  const stream = "BT /F1 18 Tf 36 90 Td (Oikome acceptance document) Tj ET\n";
  const objects = [
    "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
    "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
    "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 144] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n",
    `4 0 obj\n<< /Length ${Buffer.byteLength(stream)} >>\nstream\n${stream}endstream\nendobj\n`,
    "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
  ];
  let text = "%PDF-1.4\n";
  const offsets = [];
  for (const object of objects) {
    offsets.push(Buffer.byteLength(text));
    text += object;
  }
  const xref = Buffer.byteLength(text);
  text += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  text += offsets.map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`).join("");
  text += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return text;
}

function validPsd() {
  const psd = Buffer.alloc(43);
  psd.write("8BPS", 0);
  psd.writeUInt16BE(1, 4);
  psd.writeUInt16BE(3, 10);
  psd.writeUInt32BE(1, 12);
  psd.writeUInt32BE(1, 16);
  psd.writeUInt16BE(8, 20);
  psd.writeUInt16BE(3, 22);
  psd.writeUInt16BE(0, 38);
  psd.set([56, 102, 65], 40);
  return psd;
}

function generateFiles(root) {
  const photos = join(REPO, "docs", "prototype", "lisbon", "pick");
  for (const [source, target] of [["2-tram-of-line-28e-on-rua-augusto.jpg", "Tram 28E.jpg"], ["4-pasteis-de-belem-4128575449.jpg", "Pastéis de Belém.jpg"], ["6-sunset-in-lisbon-36542861635.jpg", "Sunset.jpg"]]) {
    const destination = resolve(root, "nas-mount", "Oikome", "photos", target);
    mkdirSync(dirname(destination), { recursive: true });
    copyFileSync(join(photos, source), destination);
  }
  writeWav(root);
  write(root, "media/route.svg", `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="360" viewBox="0 0 640 360">
    <rect width="640" height="360" fill="#edf2e9"/>
    <path d="M40 290C160 210 190 90 300 160S450 280 600 60" fill="none" stroke="#386641" stroke-width="12" stroke-linecap="round"/>
    <circle cx="40" cy="290" r="16" fill="#bc4749"/><circle cx="600" cy="60" r="16" fill="#bc4749"/>
    <text x="40" y="45" fill="#141310" font-family="sans-serif" font-size="24">Lisbon, slowly</text>
  </svg>`);
  write(root, "content-files/unsafe.svg", `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180" onload="alert(1)">
    <script>alert(1)</script><style>@import url('http://127.0.0.1:9/style.css');</style>
    <image href="http://127.0.0.1:9/pixel.png" width="320" height="180" />
    <rect width="320" height="180" fill="#bc4749" />
  </svg>`);
  write(root, "notes/Reading.md", "# Lisbon, slowly\n\nThree days on foot, with time for coffee.\n\n> Keep Thursday open for Porto.\n\n## First morning\n\n- Take tram 28 to Graça.\n- Walk down through Alfama.\n- Save the sunset for Santa Luzia.\n\n| Place | Day |\n| --- | --- |\n| Belém | Monday |\n| Sintra | Wednesday |\n");
  write(root, "content-files/example.ts", "export const greet = (name: string): string => `Hello, ${name}`;\n");
  write(root, "content-files/config.yaml", "preview:\n  lazy: true\n  formats: [markdown, images, audio, video]\n");
  write(root, "content-files/rows.csv", "day,category,amount\n2026-11-01,Train,18.40\n2026-11-02,Books,24.90\n");
  write(root, "content-files/rows.jsonl", '{"city":"Lisbon","temperature":21.4}\n{"city":"Porto","temperature":19.8}\n');
  write(root, "content-files/damaged.json", '{ "readableAs": "source", "complete": ');
  write(root, "content-files/External.pdf", validPdf());
  write(root, "content-files/Portfolio.psd", validPsd());
  const readings = ["id,recorded_at,living_room_celsius"];
  for (let hour = 0; hour < 72; hour++) {
    const recordedAt = new Date(Date.UTC(2026, 10, 1, hour)).toISOString().replace("T", " ").replace(".000Z", "");
    const temperature = (21.2 + Math.sin(hour / 7) * 1.4 + Math.cos(hour / 13) * 0.4).toFixed(1);
    readings.push(`${hour + 1},${recordedAt},${temperature}`);
  }
  write(root, "tables/readings.csv", readings.join("\n") + "\n");
}

async function startNode(root, port, home, environment) {
  const child = spawn(BIN, [
    "--root", root,
    "serve",
    "--listen", "127.0.0.1:0",
    "--p2p-listen", "/ip4/127.0.0.1/tcp/0",
    "--no-mdns",
    "--no-port-mapping",
    "--web",
    "--web-listen", `127.0.0.1:${port}`,
    "--web-root", WEB_ROOT,
  ], { stdio: ["ignore", "inherit", "inherit"], env: { ...ENV, HOME: home, ...environment } });
  const login = join(root, ".oikome", "web", "login");
  for (let i = 0; ; i++) {
    if (existsSync(login)) break;
    if (i === 200 || child.exitCode !== null) {
      child.kill();
      throw new Error(`The acceptance node did not start; is port ${port} already in use?`);
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  return { child, loginLink: readFileSync(login, "utf8").trim() };
}

async function withApi(loginLink, actions) {
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage();
    await page.goto(loginLink);
    const call = async (op, body = {}) => {
      const result = await page.evaluate(async ([op, body]) => {
        const response = await fetch(`/api/${op}`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body),
        });
        return response.json();
      }, [op, body]);
      if (result?.code) throw new Error(`${op}: ${result.message ?? result.code}`);
      return result;
    };
    return await actions(call);
  } finally {
    await browser.close();
  }
}

async function seedWorkspace(config, files, loginLink, rustfs, peerDid) {
  return withApi(loginLink, async (call) => {
    await call("rename_node", { name: config.title });
    await call("confirm_recovery_key");
    for (const collection of config.collections) {
      if (collection.action === "viewInPlace") {
        await call("view_in_place", { path: join(files, collection.path), name: collection.name });
      } else if (collection.action === "addCsvSource") {
        await call("add_source", { kind: "csv", path: join(files, collection.path) });
      } else {
        throw new Error(`Unknown acceptance action ${collection.action}`);
      }
    }

    const nas = await call("add_storage", {
      label: "NAS Mounted Folder (test double)",
      type: "folder",
      address: join(files, "nas-mount"),
      encrypted: false,
    });
    await call("view_in_place", { place: nas.name, path: "photos", name: "photos" });

    const drive = await call("add_storage", {
      label: "Drive Sync Folder (test double)",
      type: "drive",
      address: join(files, "drive-sync"),
      encrypted: true,
    });
    await call("create_space", { name: "drive-reading", storage: drive.name });
    await call("run_sql", {
      sql: 'CREATE TABLE "drive-reading".books (id BIGINT, title VARCHAR, status VARCHAR); INSERT INTO "drive-reading".books VALUES (1, \'The Book of Disquiet\', \'Reading\'), (2, \'Blindness\', \'Read\'), (3, \'Pereira Maintains\', \'Next\');',
    });

    const object = await call("add_storage", {
      label: "RustFS Acceptance",
      type: "s3",
      address: rustfs.endpoint,
      bucket: "acceptance",
      username: rustfs.accessKey,
      password: rustfs.secretKey,
      encrypted: true,
    });
    await call("create_space", { name: "rustfs-archive", storage: object.name });
    await call("run_sql", {
      sql: 'CREATE TABLE "rustfs-archive".documents (id BIGINT, title VARCHAR, year BIGINT); INSERT INTO "rustfs-archive".documents VALUES (1, \'Lisbon tram tickets\', 2024), (2, \'Porto photo contact sheet\', 2025), (3, \'Family recipes scanned\', 2026);',
    });

    await call("add_contact", { did: peerDid, name: "Acceptance Peer" });
    return call("share_space", { space: "photos", did: peerDid });
  });
}

async function seedPeer(loginLink, invite) {
  return withApi(loginLink, async (call) => {
    await call("rename_node", { name: "Acceptance Peer" });
    await call("confirm_recovery_key");
    const pending = await call("add_invite", { invite: invite.invite });
    await call("accept_invite", { id: pending.id, name: "photos-from-acceptance" });
  });
}

function freshLogin(root) {
  return readFileSync(join(root, ".oikome", "web", "login"), "utf8").trim();
}

function stopChild(child) {
  if (child.exitCode !== null) return Promise.resolve();
  child.kill();
  return once(child, "exit").then(() => {});
}

export async function launchAcceptance({ root, port = 0, env, firstRun = false } = {}) {
  const config = manifest();
  const scenarioRoot = root ?? GENERATED;
  resetDirectory(scenarioRoot, firstRun);
  const files = join(scenarioRoot, "files");
  const nodeRoot = join(scenarioRoot, "node");
  const home = join(scenarioRoot, "home");
  mkdirSync(files, { recursive: true });
  mkdirSync(home, { recursive: true });
  mkdirSync(join(files, "nas-mount"), { recursive: true });
  mkdirSync(join(files, "drive-sync"), { recursive: true });
  generateFiles(files);
  await writeWebm(files);

  const main = await startNode(nodeRoot, port, home, env);
  let peer;
  let rustfs;
  let stopped = false;
  const stop = async () => {
    if (stopped) return;
    stopped = true;
    await stopChild(main.child);
    if (peer) await stopChild(peer.child);
    rustfs?.stop();
  };
  try {
    if (firstRun) {
      const loginLink = freshLogin(nodeRoot);
      return { manifest: config, firstRun, root: scenarioRoot, nodeRoot, files, loginLink, base: new URL(loginLink).origin, stop };
    }

    rustfs = await startRustfs({ root: join(scenarioRoot, "services", "rustfs") });
    await createBucket(rustfs, "acceptance");
    const peerRoot = join(scenarioRoot, "peer");
    peer = await startNode(peerRoot, 0, join(scenarioRoot, "peer-home"), env);
    const peerDid = oikome(peerRoot, "identity").trim();
    const invite = await seedWorkspace(config, files, main.loginLink, rustfs, peerDid);
    await seedPeer(peer.loginLink, invite);

    const loginLink = freshLogin(nodeRoot);
    const peerLoginLink = freshLogin(peerRoot);
    return {
      manifest: config,
      firstRun,
      root: scenarioRoot,
      nodeRoot,
      peerRoot,
      files,
      loginLink,
      peerLoginLink,
      base: new URL(loginLink).origin,
      peerBase: new URL(peerLoginLink).origin,
      stop,
    };
  } catch (error) {
    await stop();
    throw error;
  }
}
