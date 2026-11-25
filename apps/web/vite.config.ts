// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Vite 7: Vite 8 requires lightningcss (MPL-2.0), outside the permissive
// licenses the project accepts.
//
// `npm run build` writes the app to dist/ for `oikome serve --web
// --web-root apps/web/dist`. `npm run dev` serves it with hot reload and
// sends /api, /login and /mcp to a running node (OIKOME_WEB_URL, default
// http://127.0.0.1:7340), presenting the node's own origin so its
// same-origin checks pass. Log in by opening the node's login link with
// this server's address instead of the node's.

import { createReadStream } from "node:fs";
import { cpSync, readFileSync, statSync } from "node:fs";
import { extname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const node = process.env.OIKOME_WEB_URL ?? "http://127.0.0.1:7340";
const pdfDist = fileURLToPath(new URL("../", import.meta.resolve("pdfjs-dist")));
const pdfResources = fileURLToPath(new URL("./dist/pdf/", import.meta.url));

export default defineConfig({
  plugins: [
    {
      // The explorer's page makes no connections, so it cannot fetch
      // Perspective's WebAssembly: `x.wasm?inline` puts the file in the
      // script, as a data URL the page decodes itself.
      name: "wasm-inline",
      enforce: "pre",
      load(id) {
        if (!id.endsWith(".wasm?inline")) return null;
        const bytes = readFileSync(id.slice(0, -"?inline".length));
        return `export default "data:application/wasm;base64,${bytes.toString("base64")}";`;
      },
    },
    {
      name: "pdf-preview-resources",
      configureServer(server) {
        server.middlewares.use("/pdf", (request, response, next) => {
          const path = resolve(pdfDist, `.${(request.url ?? "/").split("?")[0]}`);
          try {
            if (!path.startsWith(pdfDist) || !statSync(path).isFile()) return next();
            const types: Record<string, string> = { ".wasm": "application/wasm", ".mjs": "text/javascript", ".ttf": "font/ttf", ".pfb": "application/octet-stream" };
            response.setHeader("Content-Type", types[extname(path)] ?? "application/octet-stream");
            createReadStream(path).pipe(response);
          } catch {
            next();
          }
        });
      },
      closeBundle() {
        for (const directory of ["cmaps", "standard_fonts", "wasm", "image_decoders"]) {
          cpSync(`${pdfDist}/${directory}`, `${pdfResources}/${directory}`, { recursive: true });
        }
      },
    },
  ],
  // JSX through Vite's own esbuild; no React plugin (and so no hot refresh
  // of components in development).
  esbuild: { jsx: "automatic" },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Two pages: the app, and the explorer the Analysis view embeds
    // (Perspective), which the node serves with a policy of its own.
    rollupOptions: {
      input: {
        app: fileURLToPath(new URL("./index.html", import.meta.url)),
        explorer: fileURLToPath(new URL("./analysis-frame/index.html", import.meta.url)),
      },
    },
  },
  server: {
    proxy: {
      "/api": {
        target: node,
        changeOrigin: true,
        configure: (proxy) => {
          proxy.on("proxyReq", (request) => request.setHeader("origin", node));
        },
      },
      "/login": { target: node, changeOrigin: true },
      // The MCP Access screen builds its `claude mcp add` command from
      // this page's own address. No Origin is added: the node refuses
      // /mcp requests that carry one.
      "/mcp": { target: node, changeOrigin: true },
    },
  },
});
