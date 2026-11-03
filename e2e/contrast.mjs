// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Rendered contrast audit, run inside the page: every visible text run is
// measured against the background it actually renders on (translucent
// layers composited) and must meet WCAG AA, 4.5:1, or 3:1 for large text.
// Disabled controls are exempt, as in WCAG.

/** Run with `page.evaluate(auditContrast)`; returns the failures. */
export function auditContrast() {
  const parse = (c) => {
    const m = c && c.match(/rgba?\(([^)]+)\)/);
    if (!m) return null;
    const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number);
    return { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 };
  };
  const blend = (top, bottom) => ({
    r: top.r * top.a + bottom.r * (1 - top.a),
    g: top.g * top.a + bottom.g * (1 - top.a),
    b: top.b * top.a + bottom.b * (1 - top.a),
    a: 1,
  });
  const lin = (v) => {
    v /= 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  const lum = (c) => 0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b);
  const ratio = (a, b) => {
    const x = lum(a);
    const y = lum(b);
    return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
  };
  const backgroundOf = (el) => {
    const layers = [];
    for (let e = el; e; e = e.parentElement) {
      const c = parse(getComputedStyle(e).backgroundColor);
      if (c && c.a > 0) {
        layers.push(c);
        if (c.a >= 1) break;
      }
    }
    let base = { r: 255, g: 255, b: 255, a: 1 };
    for (let i = layers.length - 1; i >= 0; i--) base = blend(layers[i], base);
    return base;
  };
  const failures = [];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  let node;
  while ((node = walker.nextNode())) {
    const text = node.textContent.trim();
    if (!text) continue;
    const el = node.parentElement;
    if (!el || el.closest("svg, script, style, title, :disabled, [aria-disabled='true']")) continue;
    const cs = getComputedStyle(el);
    if (cs.visibility === "hidden" || cs.display === "none" || parseFloat(cs.fontSize) === 0) continue;
    const rect = el.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) continue;
    const bg = backgroundOf(el);
    const fg = blend(parse(cs.color), bg);
    const size = parseFloat(cs.fontSize);
    const bold = parseInt(cs.fontWeight, 10) >= 700;
    const need = size >= 24 || (size >= 18.66 && bold) ? 3 : 4.5;
    const r = ratio(fg, bg);
    if (r < need) failures.push(`${r.toFixed(2)} < ${need}: "${text.slice(0, 40)}" (${size}px)`);
  }
  return failures;
}
