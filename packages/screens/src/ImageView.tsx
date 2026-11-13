// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { useEffect, useRef, useState } from "react";
import PhotoSwipe from "photoswipe";
import "photoswipe/style.css";
import { ErrorText } from "@oikome/ui";
import type { ImageGallery } from "./item";
import { strings } from "./strings";

const s = strings.collections;

export default function ImageView({ src, alt, gallery }: { src: string; alt: string; gallery?: ImageGallery }) {
  const host = useRef<HTMLDivElement>(null);
  const image = useRef<HTMLImageElement>(null);
  const active = useRef<PhotoSwipe | undefined>(undefined);
  const currentGallery = useRef(gallery);
  const [error, setError] = useState(false);
  currentGallery.current = gallery;

  useEffect(() => {
    setError(false);
  }, [src]);
  useEffect(
    () => () => {
      active.current?.destroy();
      active.current = undefined;
    },
    [],
  );

  const open = () => {
    const thumbnail = image.current;
    const parent = host.current?.closest("dialog");
    if (!thumbnail?.naturalWidth || !parent || active.current) return;
    const gallery = currentGallery.current;
    const inGallery = gallery && gallery.index >= 0 && gallery.items.length > 0;
    const items = (inGallery ? gallery.items : [{ src, alt }]).map((item) => ({
      ...item,
      ...(item.src === src ? { width: thumbnail.naturalWidth, height: thumbnail.naturalHeight } : {}),
    }));
    const pswp = new PhotoSwipe({
      appendToEl: parent,
      dataSource: items,
      index: inGallery ? gallery.index : 0,
      loop: false,
      wheelToZoom: true,
      showHideAnimationType: "fade",
      errorMsg: s.imageUnavailable,
      closeTitle: strings.common.close,
      zoomTitle: s.zoomImage,
      arrowPrevTitle: s.previousItem,
      arrowNextTitle: s.nextItem,
    });
    pswp.on("contentLoad", (event) => {
      const { content, isLazy } = event;
      if ((content.data.width && content.data.height) || !content.data.src) return;
      event.preventDefault();
      const original = new Image();
      original.onload = () => {
        content.data.width = original.naturalWidth;
        content.data.height = original.naturalHeight;
        content.width = original.naturalWidth;
        content.height = original.naturalHeight;
        content.slide?.updateContentSize(true);
        content.load(isLazy);
      };
      original.onerror = () => content.onError();
      original.src = content.data.src;
    });
    pswp.on("change", () => {
      const gallery = currentGallery.current;
      if (gallery && gallery.index >= 0) gallery.open(pswp.currIndex);
    });
    pswp.on("destroy", () => {
      if (active.current === pswp) active.current = undefined;
    });
    active.current = pswp;
    pswp.init();
  };

  return (
    <div ref={host} className="screen-image-view">
      <button type="button" aria-label={s.zoomImage} title={s.zoomImage} onClick={open} disabled={error}>
        <img ref={image} src={src} alt={alt} onError={() => setError(true)} />
      </button>
      {error && <ErrorText>{s.imageUnavailable}</ErrorText>}
    </div>
  );
}
