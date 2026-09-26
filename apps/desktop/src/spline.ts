type Pt = readonly [number, number];

/**
 * SVG / Path2D path through `pts` as a smooth cubic spline (Catmull-Rom converted to
 * Bézier segments). Control points are clamped vertically between their neighbours'
 * range so the curve never overshoots (no ringing below zero on a histogram, no
 * bumps on a tone curve).
 */
export function smoothPath(pts: readonly Pt[], tension = 1): string {
  if (pts.length === 0) return "";
  const f = (v: number) => v.toFixed(2);
  if (pts.length < 3) return pts.map(([x, y], i) => `${i ? "L" : "M"}${f(x)},${f(y)}`).join("");
  let d = `M${f(pts[0][0])},${f(pts[0][1])}`;
  for (let i = 0; i < pts.length - 1; i++) {
    const p0 = pts[Math.max(0, i - 1)];
    const p1 = pts[i];
    const p2 = pts[i + 1];
    const p3 = pts[Math.min(pts.length - 1, i + 2)];
    const lo = Math.min(p1[1], p2[1]);
    const hi = Math.max(p1[1], p2[1]);
    const c = (v: number) => Math.min(hi, Math.max(lo, v));
    const t = tension / 6;
    const c1: Pt = [p1[0] + (p2[0] - p0[0]) * t, c(p1[1] + (p2[1] - p0[1]) * t)];
    const c2: Pt = [p2[0] - (p3[0] - p1[0]) * t, c(p2[1] - (p3[1] - p1[1]) * t)];
    d += `C${f(c1[0])},${f(c1[1])} ${f(c2[0])},${f(c2[1])} ${f(p2[0])},${f(p2[1])}`;
  }
  return d;
}
