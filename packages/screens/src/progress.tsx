// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// A transfer under way, as every screen shows one: a model's download, a
// backup sent to its place.

import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { bytes } from "./format";
import { strings } from "./strings";

/**
 * A transfer in progress: what it is doing, a bar, how much of how much,
 * how fast and about how long is left. The node reports only how far it
 * is; the speed comes from one reading to the next.
 */
export function TransferProgress({
  done,
  total,
  label,
  step,
  note,
  waiting,
  installing,
}: {
  done: number;
  total: number | null;
  /** What the bar is for, for screen readers. */
  label: string;
  /** Above the bar: the step it is at ("Copying the data folder"). */
  step?: ReactNode;
  /** Under the figures: where the files go. */
  note?: ReactNode;
  /** Said while there is nothing to count yet; "Connecting…" by default. */
  waiting?: string;
  installing?: boolean;
}) {
  const d = strings.knowledge.download;
  const [rate, setRate] = useState<number>();
  const last = useRef<{ at: number; done: number } | undefined>(undefined);
  useEffect(() => {
    const at = performance.now();
    const before = last.current;
    if (before && done > before.done && at > before.at) {
      const now = ((done - before.done) * 1000) / (at - before.at);
      setRate((old) => (old ? old * 0.6 + now * 0.4 : now));
    }
    last.current = { at, done };
  }, [done]);
  const pct = total ? Math.min(100, Math.floor((done / total) * 100)) : undefined;
  const parts: string[] = [];
  if (installing) {
    parts.push(d.installing);
  } else if (!done && !total) {
    parts.push(waiting ?? d.connecting);
  } else {
    parts.push(total ? d.progress(bytes(done), bytes(total)) : bytes(done));
    if (rate) {
      parts.push(d.speed(bytes(Math.round(rate))));
      if (total) {
        const left = Math.max(0, (total - done) / rate);
        parts.push(
          d.remaining(left < 60 ? d.seconds(Math.max(1, Math.round(left))) : d.minutes(Math.ceil(left / 60))),
        );
      }
    }
  }
  return (
    <div className="screen-download" role="status">
      {step && <p className="screen-download__step">{step}</p>}
      <div className="screen-download__bar">
        <progress max={100} value={installing ? undefined : pct} aria-label={label} />
        {pct !== undefined && !installing && <span className="screen-download__pct">{`${pct}%`}</span>}
      </div>
      <p className="screen-download__line">{parts.join(" · ")}</p>
      {note && (
        <p className="screen-download__path" translate="no">
          {note}
        </p>
      )}
    </div>
  );
}
