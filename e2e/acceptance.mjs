// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { oikome } from "./node.mjs";
import { launchAcceptance } from "./acceptance-node.mjs";

const args = process.argv.slice(2);
const named = (flag, fallback) => args.includes(flag) ? args[args.indexOf(flag) + 1] : fallback;
const port = Number(named("--port", "7342"));
const firstRun = args.includes("--first-run");
const open = !args.includes("--no-open");
const openPeer = args.includes("--open-peer");
const generated = join("tests", ".generated", "acceptance");

function existing() {
  const marker = join(generated, ".oikome-scenario.json");
  const login = join(generated, "node", ".oikome", "web", "login");
  if (!existsSync(marker) || !existsSync(login)) return undefined;
  const state = JSON.parse(readFileSync(marker, "utf8"));
  if (state.scenario !== "acceptance" || state.firstRun !== firstRun) {
    throw new Error("Another acceptance mode is already running; stop it before changing modes");
  }
  const link = readFileSync(login, "utf8").trim();
  if (Number(new URL(link).port) !== port) return undefined;
  return {
    link,
    nodeRoot: join(generated, "node"),
    peerRoot: join(generated, "peer"),
    peerLink: existsSync(join(generated, "peer", ".oikome", "web", "login"))
      ? readFileSync(join(generated, "peer", ".oikome", "web", "login"), "utf8").trim()
      : undefined,
  };
}

function openBrowser(nodeRoot, peerRoot) {
  oikome(nodeRoot, "web", "open");
  if (peerRoot && openPeer) oikome(peerRoot, "web", "open");
}

try {
  const running = existing();
  const response = running ? await fetch(new URL(running.link).origin, { redirect: "manual" }).catch(() => undefined) : undefined;
  if (running && response) {
    console.log(`Acceptance is already running: ${new URL(running.link).origin}`);
    if (running.peerLink) console.log(`Peer: ${new URL(running.peerLink).origin}`);
    if (open) openBrowser(running.nodeRoot, running.peerLink ? running.peerRoot : undefined);
  } else {
    const running = await launchAcceptance({ port, firstRun });
    console.log(`${running.manifest.title}: ${running.base}`);
    if (running.peerBase) console.log(`Peer: ${running.peerBase}`);
    console.log(`Isolated files: ${running.root}`);
    console.log(firstRun ? "Fresh first-run configuration" : "Fully seeded acceptance workspace");
    if (open) openBrowser(running.nodeRoot, running.peerRoot);
    console.log("Press Ctrl+C to stop acceptance. Restarting resets only its own generated directory.");
    const stop = async () => {
      await running.stop();
      process.exit(0);
    };
    process.on("SIGINT", stop);
    process.on("SIGTERM", stop);
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
}
