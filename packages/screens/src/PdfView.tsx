// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { useEffect, useRef, useState } from "react";
import { ErrorText, Icon, IconButton } from "@oikome/ui";
import { GlobalWorkerOptions, TextLayer, getDocument } from "pdfjs-dist";
import type { PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import { strings } from "./strings";

GlobalWorkerOptions.workerSrc = new URL("pdfjs-dist/build/pdf.worker.min.mjs", import.meta.url).toString();

const s = strings.collections;

function loadPdf(src: string) {
  return getDocument({
    url: src,
    disableAutoFetch: true,
    cMapUrl: "/pdf/cmaps/",
    cMapPacked: true,
    standardFontDataUrl: "/pdf/standard_fonts/",
    wasmUrl: "/pdf/wasm/",
    useWorkerFetch: true,
  });
}

export default function PdfView({ src, label }: { src: string; label: string }) {
  const host = useRef<HTMLDivElement>(null);
  const pageHost = useRef<HTMLDivElement>(null);
  const [pdf, setPdf] = useState<PDFDocumentProxy>();
  const [pageNumber, setPageNumber] = useState(1);
  const [availableWidth, setAvailableWidth] = useState(0);
  const [zoom, setZoom] = useState(1);
  const [scale, setScale] = useState(1);
  const [error, setError] = useState(false);

  useEffect(() => {
    let current = true;
    setPdf(undefined);
    setPageNumber(1);
    setZoom(1);
    setError(false);
    const task = loadPdf(src);
    void task.promise.then(
      (loaded) => {
        if (current) setPdf(loaded);
      },
      () => {
        if (current) setError(true);
      },
    );
    return () => {
      current = false;
      void task.destroy();
    };
  }, [src]);

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    const measure = () => setAvailableWidth(Math.max(1, element.clientWidth - 32));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!pdf || !pageHost.current || availableWidth <= 0) return;
    let current = true;
    let rendering: RenderTask | undefined;
    let text: TextLayer | undefined;
    const render = async () => {
      try {
        const page = await pdf.getPage(pageNumber);
        const base = page.getViewport({ scale: 1 });
        const fitted = Math.min(1, availableWidth / base.width) * zoom;
        const viewport = page.getViewport({ scale: fitted });
        const pixels = Math.min(globalThis.devicePixelRatio || 1, 2);
        const canvasViewport = page.getViewport({ scale: fitted * pixels });
        const frame = document.createElement("div");
        frame.className = "screen-pdf__page";
        frame.style.setProperty("--scale-factor", String(viewport.scale));
        frame.style.width = `${viewport.width}px`;
        frame.style.height = `${viewport.height}px`;
        const canvas = document.createElement("canvas");
        canvas.className = "screen-pdf__canvas";
        canvas.width = Math.floor(canvasViewport.width);
        canvas.height = Math.floor(canvasViewport.height);
        canvas.style.width = `${viewport.width}px`;
        canvas.style.height = `${viewport.height}px`;
        canvas.setAttribute("aria-hidden", "true");
        frame.append(canvas);
        const textHost = document.createElement("div");
        textHost.className = "screen-pdf__text";
        frame.append(textHost);
        const context = canvas.getContext("2d");
        if (!context) throw new Error("Canvas is unavailable");
        rendering = page.render({ canvas, canvasContext: context, viewport: canvasViewport });
        const content = await page.getTextContent();
        text = new TextLayer({ textContentSource: content, container: textHost, viewport });
        await Promise.all([rendering.promise, text.render()]);
        if (current) {
          pageHost.current?.replaceChildren(frame);
          setScale(fitted);
        }
      } catch {
        if (current) setError(true);
      }
    };
    void render();
    return () => {
      current = false;
      rendering?.cancel();
      text?.cancel();
    };
  }, [pdf, pageNumber, availableWidth, zoom]);

  return (
    <section ref={host} className="screen-pdf" aria-label={label}>
      {pdf && !error && (
        <div className="screen-pdf__toolbar">
          <IconButton icon="back" label={s.previousPage} disabled={pageNumber === 1} onClick={() => setPageNumber((page) => Math.max(1, page - 1))} />
          <span className="screen-pdf__position">{s.pagePosition(pageNumber, pdf.numPages)}</span>
          <IconButton icon="chevron" label={s.nextPage} disabled={pageNumber === pdf.numPages} onClick={() => setPageNumber((page) => Math.min(pdf.numPages, page + 1))} />
          <div className="screen-pdf__zoom">
            <IconButton icon="minus" label={s.zoomOut} disabled={zoom <= 0.25} onClick={() => setZoom((value) => Math.max(0.25, value / 1.25))} />
            <span className="screen-pdf__position">{Math.round(scale * 100)}%</span>
            <IconButton icon="plus" label={s.zoomIn} disabled={zoom >= 4} onClick={() => setZoom((value) => Math.min(4, value * 1.25))} />
          </div>
        </div>
      )}
      {error ? <ErrorText>{s.pdfUnavailable}</ErrorText> : <div ref={pageHost} className="screen-pdf__pages" />}
      {!pdf && !error && <div className="screen-item__skeleton oik-skel" aria-hidden="true" />}
    </section>
  );
}

export function PdfThumbnail({ src }: { src: string }) {
  const host = useRef<HTMLSpanElement>(null);
  const [near, setNear] = useState(false);

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

  useEffect(() => {
    if (!near) return;
    let current = true;
    let rendering: RenderTask | undefined;
    const task = loadPdf(src);
    const render = async () => {
      try {
        const pdf = await task.promise;
        const page = await pdf.getPage(1);
        const element = host.current;
        if (!element) return;
        const base = page.getViewport({ scale: 1 });
        const fitted = Math.max(
          0.01,
          Math.min(Math.max(1, element.clientWidth - 16) / base.width, Math.max(1, element.clientHeight - 16) / base.height),
        );
        const viewport = page.getViewport({ scale: fitted });
        const pixels = Math.min(globalThis.devicePixelRatio || 1, 1.5);
        const canvasViewport = page.getViewport({ scale: fitted * pixels });
        const canvas = document.createElement("canvas");
        canvas.className = "screen-pdf-thumbnail__canvas";
        canvas.width = Math.floor(canvasViewport.width);
        canvas.height = Math.floor(canvasViewport.height);
        canvas.style.width = `${viewport.width}px`;
        canvas.style.height = `${viewport.height}px`;
        const context = canvas.getContext("2d");
        if (!context) throw new Error("Canvas is unavailable");
        rendering = page.render({ canvas, canvasContext: context, viewport: canvasViewport });
        await rendering.promise;
        page.cleanup();
        if (current) element.replaceChildren(canvas);
      } catch {
        // A damaged or unsupported document keeps its file-kind icon.
      }
    };
    void render();
    return () => {
      current = false;
      rendering?.cancel();
      void task.destroy();
    };
  }, [src, near]);

  return (
    <span ref={host} className="screen-ftile__art screen-pdf-thumbnail" aria-hidden="true">
      <Icon name="collection" />
    </span>
  );
}
