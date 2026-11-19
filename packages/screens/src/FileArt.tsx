// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { Icon } from "@oikome/ui";
import type { IconName } from "@oikome/ui";
import type { FileItem } from "@oikome/client";
import { withoutTagLines } from "./content";

const PdfThumbnail = lazy(async () => ({ default: (await import("./PdfView")).PdfThumbnail }));
const SafeSvgThumbnail = lazy(async () => ({ default: (await import("./SafeSvgImage")).SafeSvgThumbnail }));

export function headRows(name: string, text: string): string[][] {
  const separator = name.toLowerCase().endsWith(".tsv") ? "\t" : ",";
  return text
    .split("\n")
    .filter((line) => line.trim())
    .slice(0, 3)
    .map((line) => {
      const cells: string[] = [];
      let cell = "";
      let quoted = false;
      for (let i = 0; i < line.length; i++) {
        const c = line[i]!;
        if (quoted) {
          if (c === '"' && line[i + 1] === '"') {
            cell += '"';
            i++;
          } else if (c === '"') quoted = false;
          else cell += c;
        } else if (c === '"') quoted = true;
        else if (c === separator) {
          cells.push(cell);
          cell = "";
        } else cell += c;
      }
      cells.push(cell);
      return cells.slice(0, 3);
    });
}

export function fileIcon(file: FileItem): IconName {
  if (file.content.views.includes("audio")) return "audio";
  if (file.content.views.includes("video")) return "video";
  if (file.kind === "image") return "photo";
  if (file.kind === "note") return "note";
  if (file.kind === "table") return "table";
  return "collection";
}

function VideoThumbnail({ src }: { src: string }) {
  const host = useRef<HTMLSpanElement>(null);
  const [near, setNear] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    if (typeof IntersectionObserver === "undefined") {
      setNear(true);
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((entry) => entry.isIntersecting)) return;
        setNear(true);
        observer.disconnect();
      },
      { rootMargin: "240px" },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [src]);

  return (
    <span ref={host} className="screen-ftile__art screen-ftile__art--icon screen-video-thumbnail" aria-hidden="true">
      {near && !failed ? (
        <video src={src} preload="metadata" muted playsInline onError={() => setFailed(true)} />
      ) : (
        <Icon name="video" />
      )}
    </span>
  );
}

export function FileArt({ file, source, image }: { file: FileItem; source: string; image?: string }) {
  const fallback = (
    <span className="screen-ftile__art screen-ftile__art--icon">
      <Icon name={fileIcon(file)} />
    </span>
  );
  if (image) {
    return (
      <span className="screen-ftile__art">
        <img src={image} alt="" decoding="async" loading="lazy" />
      </span>
    );
  }
  if (file.content.views.includes("pdf")) {
    return (
      <Suspense fallback={fallback}>
        <PdfThumbnail src={source} />
      </Suspense>
    );
  }
  if (file.content.format === "svg" && file.content.views.includes("image")) {
    return (
      <span className="screen-ftile__art screen-ftile__art--icon screen-svg-thumbnail">
        <Suspense fallback={<Icon name="photo" />}>
          <SafeSvgThumbnail src={source} />
        </Suspense>
      </span>
    );
  }
  if (file.content.views.includes("video")) return <VideoThumbnail src={source} />;
  if (file.content.views.includes("audio")) {
    return (
      <span className="screen-ftile__art screen-ftile__art--icon screen-audio-thumbnail">
        <Icon name="audio" />
      </span>
    );
  }
  if (file.kind === "note" && file.text != null) {
    return (
      <span className="screen-ftile__art screen-ftile__art--paper">
        <span className="screen-ftile__text">{withoutTagLines(file.text)}</span>
      </span>
    );
  }
  const rows = file.kind === "table" && file.text ? headRows(file.name, file.text) : [];
  if (rows.length > 0) {
    return (
      <span className="screen-ftile__art screen-ftile__art--rows">
        <table className="screen-cover__table" aria-hidden="true">
          <tbody>
            {rows.map((row, r) => (
              <tr key={r}>
                {row.map((value, c) => (
                  <td key={c}>{value || "—"}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </span>
    );
  }
  return fallback;
}
