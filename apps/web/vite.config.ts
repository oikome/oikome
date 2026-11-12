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
      // The AI Assistants screen builds its `claude mcp add` command from
      // this page's own address. No Origin is added: the node refuses
      // /mcp requests that carry one.
      "/mcp": { target: node, changeOrigin: true },
    },
  },
});
