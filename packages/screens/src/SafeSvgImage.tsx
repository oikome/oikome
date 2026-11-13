// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { lazy, Suspense, useEffect, useState } from "react";
import DOMPurify from "dompurify";
import { ErrorText } from "@oikome/ui";
import type { ImageGallery } from "./item";
import { errorMessage } from "./data";
import { strings } from "./strings";

const ImageView = lazy(() => import("./ImageView"));
const s = strings.collections;
const BLOCKED = ["script", "foreignobject", "iframe", "object", "embed", "image", "feimage", "style", "a", "animate", "animatemotion", "animatetransform", "set"];
const URL_ATTRIBUTES = new Set(["href", "xlink:href", "src", "poster", "data", "action", "formaction"]);
const LOCAL_URL = /^url\(\s*['"]?#[^)'"]+['"]?\s*\)$/i;

function sanitizedSvg(source: string): string {
  const clean = DOMPurify.sanitize(source, {
    USE_PROFILES: { svg: true, svgFilters: true },
    FORBID_TAGS: BLOCKED,
    FORBID_ATTR: ["style", "srcset"],
  });
  const document = new DOMParser().parseFromString(clean, "image/svg+xml");
  if (document.querySelector("parsererror") || document.documentElement.localName !== "svg") {
    throw new Error("invalid SVG");
  }
  for (const element of Array.from(document.querySelectorAll("*"))) {
    for (const attribute of Array.from(element.attributes)) {
      const name = attribute.name.toLowerCase();
      const value = attribute.value.trim();
      if (
        name.startsWith("on") ||
        (URL_ATTRIBUTES.has(name) && !value.startsWith("#")) ||
        (value.includes("url(") && !LOCAL_URL.test(value))
      ) {
        element.removeAttribute(attribute.name);
      }
    }
  }
  return new XMLSerializer().serializeToString(document);
}

export default function SafeSvgImage({ src, alt, gallery }: { src: string; alt: string; gallery?: ImageGallery }) {
  const [url, setUrl] = useState<string>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    let live = true;
    let objectUrl: string | undefined;
    setUrl(undefined);
    setError(undefined);
    void (async () => {
      try {
        const response = await fetch(src, { credentials: "same-origin" });
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        const svg = sanitizedSvg(await response.text());
        objectUrl = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
        if (live) setUrl(objectUrl);
        else URL.revokeObjectURL(objectUrl);
      } catch (reason) {
        if (live) setError(errorMessage(reason));
      }
    })();
    return () => {
      live = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [src]);

  return (
    <div className="screen-svg-view">
      {!url && !error && <div className="screen-item__skeleton oik-skel" aria-hidden="true" />}
      {error && <ErrorText>{s.svgUnavailable}</ErrorText>}
      {url && (
        <Suspense fallback={<div className="screen-item__skeleton oik-skel" aria-hidden="true" />}>
          <ImageView src={url} alt={alt} gallery={gallery} />
        </Suspense>
      )}
    </div>
  );
}
