// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The web app: the shared screens over the node's HTTP API, on the origin
// the node serves the app from.

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { HttpClient } from "@oikome/client";
import { App } from "@oikome/screens";
import "@oikome/ui/tokens.css";
import "@oikome/ui/ui.css";
import "@oikome/screens/screens.css";

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App client={new HttpClient()} />
    </StrictMode>,
  );
}
