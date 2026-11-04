// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The Oikome plugin SDK: the bridge between a dashboard plugin and the app.
//
// A plugin is a small web page that Home shows in a card. It runs in a
// sandboxed frame with no network, no cookies and no storage, and it sees
// only the collections its owner let it read: the app reads them through
// the node, as the plugin, and sends them here. The node serves this file
// as the first script of every plugin page, before the plugin's own.
//
//   oikome.onData(({ collections, theme, today }) => { ... })
//     collections: [{ name, columns: [{ name, type }], rows: [{ ... }] }]
//       type: "text" | "number" | "date" | "image" | "bool" | "other";
//       a date is ISO text ("2025-11-07" or "2025-11-07T09:40"), an image a
//       data: URL.
//     theme: "ink" | "bold"; the page's CSS variables (--oik-ink, --oik-bg,
//       --oik-raised, --oik-line, --oik-accent, --oik-font, --oik-mono,
//       --oik-display, --oik-radius...) are set to match it.
//     today: ISO date of the owner's today.
//   oikome.open(name)  opens one of the plugin's collections in the app.
(() => {
  "use strict";

  // WebRTC is the one way out that a content security policy doesn't
  // close: it goes before any plugin code can take it.
  const rtc = [
    "RTCPeerConnection",
    "webkitRTCPeerConnection",
    "RTCDataChannel",
    "RTCIceTransport",
    "RTCSctpTransport",
    "RTCDtlsTransport",
    "RTCRtpSender",
    "RTCRtpReceiver",
    "RTCRtpTransceiver",
    "RTCSessionDescription",
    "RTCIceCandidate",
    "RTCEncodedAudioFrame",
    "RTCEncodedVideoFrame",
  ];
  for (const name of rtc) {
    try {
      Object.defineProperty(window, name, { value: undefined, writable: false, configurable: false });
    } catch {
      // Not there in this browser.
    }
  }

  const listeners = [];
  let last = null;
  const loadedFonts = new Set();

  function applyTheme(theme) {
    if (!theme || typeof theme !== "object") return;
    const root = document.documentElement;
    root.dataset.theme = theme.name === "bold" ? "bold" : "ink";
    for (const [name, value] of Object.entries(theme.vars || {})) {
      if (/^--oik-[a-z0-9-]+$/.test(name) && typeof value === "string" && value.length < 400) {
        root.style.setProperty(name, value);
      }
    }
    for (const font of theme.fonts || []) {
      const key = `${font.family}|${font.url}|${font.weight}|${font.style}`;
      if (loadedFonts.has(key) || typeof font.url !== "string" || !font.url.startsWith("/")) continue;
      loadedFonts.add(key);
      try {
        const face = new FontFace(font.family, `url(${JSON.stringify(font.url)})`, {
          weight: String(font.weight || "400"),
          style: font.style || "normal",
        });
        face.load().then((loaded) => document.fonts.add(loaded), () => {});
      } catch {
        // The plugin falls back to the system's fonts.
      }
    }
  }

  function view(message) {
    return Object.freeze({
      collections: Array.isArray(message.collections) ? message.collections : [],
      theme: message.theme && message.theme.name === "bold" ? "bold" : "ink",
      today: typeof message.today === "string" ? message.today : new Date().toISOString().slice(0, 10),
    });
  }

  window.addEventListener("message", (event) => {
    if (event.source !== window.parent) return;
    const message = event.data;
    if (!message || message.oikome !== 1) return;
    if (message.type === "data") {
      last = message;
      applyTheme(message.theme);
      for (const listener of listeners) {
        try {
          listener(view(message));
        } catch (e) {
          console.error(e);
        }
      }
    } else if (message.type === "theme") {
      applyTheme(message.theme);
    }
  });

  const oikome = Object.freeze({
    /** Call `listener` with the plugin's data now (if it came) and whenever it changes. */
    onData(listener) {
      if (typeof listener !== "function") return;
      listeners.push(listener);
      if (last) listener(view(last));
    },
    /** Open one of the plugin's collections in the app. */
    open(name) {
      window.parent.postMessage({ oikome: 1, type: "open", collection: String(name) }, "*");
    },
  });
  Object.defineProperty(window, "oikome", { value: oikome, writable: false, configurable: false });
  window.parent.postMessage({ oikome: 1, type: "ready" }, "*");
})();
