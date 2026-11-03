// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Vite 7: Vite 8 requires lightningcss (MPL-2.0), outside the permissive
// licenses the project accepts.
//
// `npm run build` writes the app to dist/ for `oikome serve --web
// --web-root apps/web/dist`. `npm run dev` serves it with hot reload and
// sends /api and /login to a running node (OIKOME_WEB_URL, default
// http://127.0.0.1:7340), presenting the node's own origin so its
// same-origin checks pass. Log in by opening the node's login link with
// this server's address instead of the node's.

import { defineConfig } from "vite";

const node = process.env.OIKOME_WEB_URL ?? "http://127.0.0.1:7340";

export default defineConfig({
  // JSX through Vite's own esbuild; no React plugin (and so no hot refresh
  // of components in development).
  esbuild: { jsx: "automatic" },
  build: {
    outDir: "dist",
    emptyOutDir: true,
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
    },
  },
});
