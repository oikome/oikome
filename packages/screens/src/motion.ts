// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Motion the screens share. A page plays it once, when it comes in; data
// read again later changes in place, without playing it a second time.
// With reduced motion asked for, everything lands where it ends.

import { useEffect, useRef, useState } from "react";

/** Whether this browser asks for less motion. */
export function reducedMotion(): boolean {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

/** The prototype's ease: quick to start, slow to settle. */
export const EASE = "cubic-bezier(0.2, 0.8, 0.2, 1)";

/**
 * A number that counts up from zero the first time it is known, over
 * `ms`, slowing as it lands; later changes show at once.
 */
export function useCountUp(target: number | undefined, ms = 700): number | undefined {
  const [shown, setShown] = useState<number>();
  const counted = useRef(false);
  useEffect(() => {
    if (target === undefined) return;
    if (counted.current || reducedMotion()) {
      counted.current = true;
      setShown(target);
      return;
    }
    const start = performance.now();
    let frame = requestAnimationFrame(function step(now) {
      const p = Math.min(1, (now - start) / ms);
      setShown(Math.round(target * (1 - Math.pow(1 - p, 3))));
      if (p < 1) frame = requestAnimationFrame(step);
      else counted.current = true;
    });
    return () => cancelAnimationFrame(frame);
  }, [target, ms]);
  return target === undefined ? undefined : (shown ?? 0);
}
