// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { spawn } from "node:child_process";
import { once } from "node:events";
import {
  chmodSync,
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { crc32 } from "node:zlib";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";
import { BIN, ENV, EXTENSIONS, oikome } from "./node.mjs";
import { createBucket, putObject, startRustfs } from "./rustfs.mjs";

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

/** A zip of `entries` ({name: text}), stored as is, as office files are
 * zips; only what a reader needs. */
function storedZip(entries) {
  const parts = [];
  const central = [];
  let offset = 0;
  for (const [name, text] of Object.entries(entries)) {
    const nameBytes = Buffer.from(name, "utf8");
    const data = Buffer.from(text, "utf8");
    const crc = crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x0800, 6);
    local.writeUInt16LE(0, 8);
    local.writeUInt16LE(0, 10);
    local.writeUInt16LE(0x21, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBytes.length, 26);
    local.writeUInt16LE(0, 28);
    const header = Buffer.alloc(46);
    header.writeUInt32LE(0x02014b50, 0);
    header.writeUInt16LE(20, 4);
    header.writeUInt16LE(20, 6);
    header.writeUInt16LE(0x0800, 8);
    header.writeUInt16LE(0, 10);
    header.writeUInt16LE(0, 12);
    header.writeUInt16LE(0x21, 14);
    header.writeUInt32LE(crc, 16);
    header.writeUInt32LE(data.length, 20);
    header.writeUInt32LE(data.length, 24);
    header.writeUInt16LE(nameBytes.length, 28);
    header.writeUInt32LE(offset, 42);
    parts.push(local, nameBytes, data);
    central.push(header, nameBytes);
    offset += local.length + nameBytes.length + data.length;
  }
  const directory = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(Object.keys(entries).length, 8);
  end.writeUInt16LE(Object.keys(entries).length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...parts, directory, end]);
}

/** An Excel workbook as Excel saves one: `sheets` maps a sheet's name to
 * its rows, each a list of cells already in Excel's XML (text inline, dates
 * in style 1, Excel's short date, formulas with the values last
 * calculated). */
function xlsx(sheets) {
  const main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
  const rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
  const names = Object.keys(sheets);
  const entries = {
    "[Content_Types].xml":
      `<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>` +
      names.map((_, i) => `<Override PartName="/xl/worksheets/sheet${i + 1}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>`).join("") +
      "</Types>",
    "_rels/.rels": `<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="${rel}/officeDocument" Target="xl/workbook.xml"/></Relationships>`,
    "xl/workbook.xml": `<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="${main}" xmlns:r="${rel}"><sheets>${names.map((name, i) => `<sheet name="${name}" sheetId="${i + 1}" r:id="rId${i + 1}"/>`).join("")}</sheets></workbook>`,
    "xl/_rels/workbook.xml.rels":
      `<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">` +
      names.map((_, i) => `<Relationship Id="rId${i + 1}" Type="${rel}/worksheet" Target="worksheets/sheet${i + 1}.xml"/>`).join("") +
      `<Relationship Id="rId${names.length + 1}" Type="${rel}/styles" Target="styles.xml"/></Relationships>`,
    "xl/styles.xml": `<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="${main}"><cellXfs count="2"><xf numFmtId="0"/><xf numFmtId="14" applyNumberFormat="1"/></cellXfs></styleSheet>`,
  };
  names.forEach((name, i) => {
    const rows = sheets[name].map((cells, r) => `<row r="${r + 1}">${cells.join("")}</row>`).join("");
    entries[`xl/worksheets/sheet${i + 1}.xml`] = `<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="${main}"><sheetData>${rows}</sheetData></worksheet>`;
  });
  return storedZip(entries);
}

/** A household's utility bills as the owner keeps them in Excel: a sheet
 * per utility, the cost a formula of the use. */
function utilities() {
  const text = (ref, value) => `<c r="${ref}" t="inlineStr"><is><t>${value}</t></is></c>`;
  // 1 September, October and November 2026 as Excel's day numbers.
  const months = [46266, 46296, 46327];
  const sheet = (unit, price, uses) => [
    [text("A1", "Month"), text("B1", unit), text("C1", "Cost")],
    ...uses.map((use, i) => {
      const r = i + 2;
      return [
        `<c r="A${r}" s="1"><v>${months[i]}</v></c>`,
        `<c r="B${r}"><v>${use}</v></c>`,
        `<c r="C${r}"><f>B${r}*${price}</f><v>${Number((use * price).toFixed(4))}</v></c>`,
      ];
    }),
  ];
  return xlsx({
    Electricity: sheet("kWh", 0.21, [182, 205, 241]),
    Water: sheet("m3", 2.35, [7.4, 6.9, 7.1]),
  });
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
  const first = "BT /F1 18 Tf 36 90 Td (Oikome acceptance document) Tj ET\n";
  const second = "BT /F1 18 Tf 36 90 Td (A second page) Tj ET\n";
  const third = "BT /F2 18 Tf 36 90 Td <4E2D6587988489C8> Tj ET\n";
  const objects = [
    "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
    "2 0 obj\n<< /Type /Pages /Kids [3 0 R 5 0 R 8 0 R] /Count 3 >>\nendobj\n",
    "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 144] /Contents 4 0 R /Resources << /Font << /F1 7 0 R >> >> >>\nendobj\n",
    `4 0 obj\n<< /Length ${Buffer.byteLength(first)} >>\nstream\n${first}endstream\nendobj\n`,
    "5 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 144] /Contents 6 0 R /Resources << /Font << /F1 7 0 R >> >> >>\nendobj\n",
    `6 0 obj\n<< /Length ${Buffer.byteLength(second)} >>\nstream\n${second}endstream\nendobj\n`,
    "7 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
    "8 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 144] /Contents 9 0 R /Resources << /Font << /F2 10 0 R >> >> >>\nendobj\n",
    `9 0 obj\n<< /Length ${Buffer.byteLength(third)} >>\nstream\n${third}endstream\nendobj\n`,
    "10 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light /Encoding /UniGB-UCS2-H /DescendantFonts [11 0 R] >>\nendobj\n",
    "11 0 obj\n<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 2 >> /FontDescriptor 12 0 R >>\nendobj\n",
    "12 0 obj\n<< /Type /FontDescriptor /FontName /STSong-Light /Flags 6 /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 900 /Descent -200 /CapHeight 700 /StemV 80 >>\nendobj\n",
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

/**
 * The project's own documents as a knowledge base: the agent protocol and
 * every design document, copied as they are at launch so the workspace never
 * writes into the repository.
 */
function copyProjectDocs(root) {
  const target = resolve(root, "oikome-docs");
  mkdirSync(target, { recursive: true });
  copyFileSync(join(REPO, "AGENT.md"), join(target, "AGENT.md"));
  const copy = (from, to) => {
    for (const entry of readdirSync(from, { withFileTypes: true })) {
      if (entry.isDirectory() && entry.name !== "prototype") copy(join(from, entry.name), join(to, entry.name));
      else if (entry.isFile() && entry.name.endsWith(".md")) {
        mkdirSync(to, { recursive: true });
        copyFileSync(join(from, entry.name), join(to, entry.name));
      }
    }
  };
  copy(join(REPO, "docs"), join(target, "docs"));
}

function generateFiles(root) {
  copyProjectDocs(root);
  const photos = join(REPO, "docs", "prototype", "lisbon", "pick");
  for (const [source, target] of [["2-tram-of-line-28e-on-rua-augusto.jpg", "Tram 28E.jpg"], ["4-pasteis-de-belem-4128575449.jpg", "Pastéis de Belém.jpg"], ["6-sunset-in-lisbon-36542861635.jpg", "Sunset.jpg"]]) {
    const destination = resolve(root, "nas-mount", "Oikome", "photos", target);
    mkdirSync(dirname(destination), { recursive: true });
    copyFileSync(join(photos, source), destination);
  }
  // A camera's RAW file beside them, as TIFF writes one: a directory that
  // says where the picture the camera stored is, then that picture. The RAW
  // Preview plugin shows it.
  const stored = readFileSync(join(photos, "6-sunset-in-lisbon-36542861635.jpg"));
  const entry = (tag, kind, value) => {
    const bytes = Buffer.alloc(12);
    bytes.writeUInt16LE(tag, 0);
    bytes.writeUInt16LE(kind, 2);
    bytes.writeUInt32LE(1, 4);
    bytes.writeUInt32LE(value, 8);
    return bytes;
  };
  const header = Buffer.from([0x49, 0x49, 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, 0x03, 0x00]);
  const raw = Buffer.concat([header, entry(0x0112, 3, 1), entry(0x0201, 4, 50), entry(0x0202, 4, stored.length), Buffer.alloc(4), stored]);
  writeFileSync(resolve(root, "nas-mount", "Oikome", "photos", "DSC_0042.NEF"), raw);
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
  write(root, "content-files/Plain.txt", "Plain text can be edited and saved as a copy.\n");
  write(root, "content-files/Read only.txt", "The original stays untouched. Save a copy to keep changes.\n");
  chmodSync(join(root, "content-files", "Read only.txt"), 0o444);
  write(root, "content-files/config.yaml", "preview:\n  lazy: true\n  formats: [markdown, images, audio, video]\n");
  write(root, "content-files/rows.csv", "day,category,amount\n2026-11-01,Train,18.40\n2026-11-02,Books,24.90\n");
  write(root, "content-files/rows.jsonl", '{"city":"Lisbon","temperature":21.4}\n{"city":"Porto","temperature":19.8}\n');
  write(root, "content-files/damaged.json", '{ "readableAs": "source", "complete": ');
  write(root, "content-files/External.pdf", validPdf());
  write(root, "content-files/Broken.pdf", "%PDF-1.4\nthis is not a complete document\n");
  write(root, "content-files/Portfolio.psd", validPsd());
  // A household's own records, as files in one folder: what Data Analysis
  // is tried on. Everything is made the way an owner would have it.
  const readings = ["recorded_at,room,celsius"];
  for (let hour = 0; hour < 72; hour++) {
    const recordedAt = new Date(Date.UTC(2026, 10, 1, hour)).toISOString().replace("T", " ").replace(".000Z", "");
    for (const [room, base] of [["Living room", 21.2], ["Bedroom", 19.6]]) {
      readings.push(`${recordedAt},${room},${(base + Math.sin(hour / 7) * 1.4 + Math.cos(hour / 13) * 0.4).toFixed(1)}`);
    }
  }
  write(root, "household/readings.csv", readings.join("\n") + "\n");
  const spends = [["Groceries", 38], ["Transport", 12], ["Eating out", 27], ["Utilities", 64], ["Books", 19], ["Health", 22]];
  for (const month of [9, 10, 11]) {
    const rows = ["date,category,amount,note"];
    for (let day = 1; day <= 28; day++) {
      const [category, base] = spends[(day * 5 + month) % spends.length];
      const amount = (base + ((day * 37 + month * 11) % 23) - 8 + ((day * 13) % 100) / 100).toFixed(2);
      rows.push(`2026-${String(month).padStart(2, "0")}-${String(day).padStart(2, "0")},${category},${amount},${category === "Books" ? "Bertrand" : ""}`);
    }
    write(root, `household/spending/2026-${String(month).padStart(2, "0")}.csv`, rows.join("\n") + "\n");
  }
  const steps = [];
  for (let day = 1; day <= 30; day++) {
    steps.push(JSON.stringify({ date: `2026-11-${String(day).padStart(2, "0")}`, steps: 5200 + ((day * 977) % 6400), city: day % 6 === 0 ? "Porto" : "Lisbon" }));
  }
  write(root, "household/steps.jsonl", steps.join("\n") + "\n");
  write(root, "household/utilities.xlsx", utilities());
  // A reading list kept in the folder a cloud drive's client syncs.
  write(root, "drive-sync/Oikome/reading-list/books.csv", "title,author,status,finished\nThe Book of Disquiet,Fernando Pessoa,Reading,\nBlindness,José Saramago,Read,2026-08-14\nPereira Maintains,Antonio Tabucchi,Next,\n");
}

/** Datasets another program keeps (PR 31): Iceberg tables of an energy
 * meter's export, which the owner keeps in the household folder, and of the
 * trams taken, kept in the archive on RustFS; and a sleep tracker's Lance
 * dataset in the household folder. A data center of their own writes them
 * (`oikome sql`, as any program writes Iceberg and Lance); they are then
 * copied where the owner keeps them, as a moved dataset is. DuckDB's
 * extensions come from EXTENSIONS, fetched there the first time
 * (`extensions install` skips what is there), and the acceptance nodes read
 * the datasets with them. Returns the trams' folder, for the bucket. */
function writeDatasets(scenarioRoot, files) {
  const writer = join(scenarioRoot, "services", "writer");
  rmSync(writer, { recursive: true, force: true });
  mkdirSync(writer, { recursive: true });
  oikome(writer, "--extension-dir", EXTENSIONS, "extensions", "install", "lance", "iceberg");
  const days = (month, base) =>
    Array.from({ length: 28 }, (_, i) => {
      const day = `2026-${String(month).padStart(2, "0")}-${String(i + 1).padStart(2, "0")}`;
      const kwh = (base + ((i * 7 + month) % 9) * 0.6).toFixed(1);
      return `(DATE '${day}', ${kwh}, ${(kwh * 0.21).toFixed(2)})`;
    }).join(", ");
  oikome(
    writer,
    "sql",
    "CREATE SPACE exports; " +
      "CREATE TABLE exports.meter (read_on DATE, kwh DOUBLE, cost DOUBLE); " +
      `INSERT INTO exports.meter VALUES ${days(10, 8)}; ` +
      `INSERT INTO exports.meter VALUES ${days(11, 9)}; ` +
      "CREATE TABLE exports.trams (taken_on DATE, line VARCHAR, fare DOUBLE); " +
      "INSERT INTO exports.trams VALUES (DATE '2026-11-03', '28E', 3.10), (DATE '2026-11-04', '15E', 3.10), " +
      "(DATE '2026-11-07', '12E', 3.10), (DATE '2026-11-12', '28E', 3.10)",
  );
  const nights = Array.from({ length: 28 }, (_, i) => {
    const night = `2026-11-${String(i + 1).padStart(2, "0")}`;
    const hours = (6.4 + ((i * 5) % 9) * 0.25).toFixed(2);
    return `(DATE '${night}', ${hours}, ${(hours * 0.22).toFixed(2)}, ${(i * 3) % 4})`;
  }).join(", ");
  oikome(
    writer,
    "--extension-dir", EXTENSIONS,
    "sql",
    "CREATE SPACE tracker WITH FORMAT = 'lance'; " +
      "CREATE TABLE tracker.sleep (night DATE, hours DOUBLE, deep_hours DOUBLE, awakenings INTEGER); " +
      `INSERT INTO tracker.sleep VALUES ${nights}`,
  );
  cpSync(join(writer, "exports", "meter"), join(files, "household", "energy-meter"), { recursive: true });
  cpSync(join(writer, ".oikome", "lance", "tracker", "sleep.lance"), join(files, "household", "sleep.lance"), { recursive: true });
  return join(writer, "exports", "trams");
}

/** Every file under `dir`, by its path within it. */
function filesUnder(dir, at = "") {
  return readdirSync(join(dir, at), { withFileTypes: true }).flatMap((entry) => {
    const path = at ? `${at}/${entry.name}` : entry.name;
    return entry.isDirectory() ? filesUnder(dir, path) : [path];
  });
}

/** What the acceptance bucket holds before Oikome sees it: a folder with
 * subfolders and mixed files, put there as any S3 client would. */
async function fillBucket(rustfs) {
  const files = {
    "archive/documents.csv": "id,title,year,kind\n1,Lisbon tram tickets,2024,ticket\n2,Porto photo contact sheet,2025,photo\n3,Family recipes scanned,2026,scan\n",
    "archive/scans/Family recipes.pdf": validPdf(),
    "archive/tickets/2024/Tram 28E.txt": "Carris · Tram 28E · Martim Moniz → Campo de Ourique · 2024-05-11 · EUR 3.10\n",
    "archive/tickets/2024/prices.csv": "date,line,price\n2024-05-11,28E,3.10\n2024-05-12,15E,3.10\n2024-05-13,Metro,1.85\n",
    "archive/README.md": "# Archive\n\nScans and tickets kept on RustFS. Oikome views this folder where it is.\n",
    // Last year's receipts, moved off the computer to the bucket.
    "household/receipts-2024/receipts.csv":
      "date,category,amount,shop\n2024-10-03,Groceries,41.20,Pingo Doce\n2024-10-17,Books,18.90,Bertrand\n2024-11-05,Groceries,36.75,Mercado de Campo de Ourique\n2024-12-12,Eating out,27.40,Cervejaria Ramiro\n",
  };
  for (const [key, contents] of Object.entries(files)) await putObject(rustfs, "acceptance", key, contents);
}

async function startNode(root, port, home, environment) {
  const child = spawn(BIN, [
    "--root", root,
    "--extension-dir", EXTENSIONS,
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
    await call("view_in_place", { place: drive.name, path: "reading-list", name: "reading-list" });

    const object = await call("add_storage", {
      label: "RustFS Acceptance",
      type: "s3",
      address: rustfs.endpoint,
      bucket: "acceptance",
      username: rustfs.accessKey,
      password: rustfs.secretKey,
      encrypted: true,
    });
    // The folder in the bucket, as the RustFS data source adds it.
    await call("view_in_place", { place: object.name, path: "archive", name: "rustfs-archive" });
    // Last year's receipts in the bucket, added to the household's own
    // collection by reference: a folder of it, read where it is.
    await call("add_in_place_folder", { space: "household", path: "household/receipts-2024", place: object.name });

    // Backups to a folder standing for an external disk, every day at 03:00;
    // one is made now, so Restore from Backup has something to offer.
    const disk = join(files, "backup-disk");
    mkdirSync(disk, { recursive: true });
    await call("set_backup", {
      destinations: [{ path: disk }],
      schedule: { every: "daily", hour: 3, minute: 0 },
    });
    await call("back_up_now");
    for (let i = 0; i < 600 && (await call("backup_info")).running; i++) {
      await new Promise((resolve) => setTimeout(resolve, 200));
    }

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
  const trams = firstRun ? undefined : writeDatasets(scenarioRoot, files);

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
    await fillBucket(rustfs);
    for (const path of filesUnder(trams)) {
      await putObject(rustfs, "acceptance", `archive/transit/${path}`, readFileSync(join(trams, path)));
    }
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
