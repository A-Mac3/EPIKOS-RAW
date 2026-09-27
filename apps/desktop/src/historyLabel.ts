import type { Adjustments, StyleInfo } from "./types";

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const signed = (v: number, digits = 0) => `${v > 0 ? "+" : ""}${v.toFixed(digits)}`;

const TONE: Record<string, string> = {
  contrast: "Contrast",
  highlights: "Highlights",
  shadows: "Shadows",
  whites: "Whites",
  blacks: "Blacks",
  vibrance: "Vibrance",
  saturation: "Saturation",
  dehaze: "Dehaze",
};

/** A short name for what changed between two edits (for the History panel). */
export function describeChange(before: Adjustments, after: Adjustments, styles: StyleInfo[] = []): string {
  const parts: string[] = [];
  const styleName = (id: string) => styles.find((s) => s.id === id)?.name ?? id;

  if (before.exposure !== after.exposure) parts.push(`Exposure ${signed(after.exposure, 2)} EV`);
  for (const [key, name] of Object.entries(TONE)) {
    const k = key as keyof Adjustments["tone"];
    if (before.tone[k] !== after.tone[k]) parts.push(`${name} ${signed(after.tone[k])}`);
  }
  if (!same(before.whiteBalance, after.whiteBalance)) {
    parts.push(
      after.whiteBalance.mode === "asShot"
        ? "White balance: As Shot"
        : `White balance ${Math.round(after.whiteBalance.temperature)} K`,
    );
  }
  if (before.lens.rotation !== after.lens.rotation || before.lens.vertical !== after.lens.vertical) {
    parts.push("Straighten / perspective");
  } else if (!same(before.lens, after.lens)) {
    parts.push(before.lens.profile !== after.lens.profile ? `Lens profile ${after.lens.profile ? "on" : "off"}` : "Lens corrections");
  }
  if (!same(before.crop, after.crop)) {
    const c = after.crop;
    parts.push(c.width >= 0.9999 && c.height >= 0.9999 ? "Crop reset" : `Crop${c.aspect !== "free" ? ` ${c.aspect}` : ""}`);
  }
  if (before.highlightRecovery !== after.highlightRecovery) parts.push("Highlight recovery");
  if (!same(before.noiseReduction, after.noiseReduction)) parts.push("Noise reduction");
  if (!same(before.local, after.local)) {
    const refined = before.local.length === after.local.length &&
      after.local.every((l, i) => {
        const { grow, feather, refine, ...rest } = l;
        const { grow: g, feather: f, refine: r, ...was } = before.local[i];
        return same(rest, was) && !same([grow, feather, refine], [g, f, r]) ? true : same(l, before.local[i]);
      });
    parts.push(
      after.local.length > before.local.length
        ? "Local adjustment added"
        : after.local.length < before.local.length
          ? "Local adjustment removed"
          : refined
            ? "Mask refined"
            : "Local adjustment",
    );
  }
  if (!same(before.retouch, after.retouch)) {
    const r0 = before.retouch;
    const r1 = after.retouch;
    parts.push(
      r1.erase.length > r0.erase.length
        ? "Generative erase"
        : r1.erase.length < r0.erase.length
          ? "Erase restored"
          : r1.spots.length !== r0.spots.length
            ? "Spot healing"
            : r1.redEye
              ? "Red-eye removal"
              : "Red-eye removal off",
    );
  }
  if (!same(before.manual, after.manual)) {
    const n = (a: Adjustments) => a.manual.length;
    const kind = (after.manual[after.manual.length - 1] ?? before.manual[before.manual.length - 1])?.shape.kind;
    const word = kind === "brush" ? "Brush" : kind === "linear" ? "Linear gradient" : "Radial gradient";
    parts.push(n(after) > n(before) ? `${word} added` : n(after) < n(before) ? "Mask removed" : "Manual mask");
  }
  if (!same(before.texture, after.texture)) parts.push("Texture & retouching");
  if (!same(before.color, after.color)) parts.push("Colour grading");
  if (!same(before.atmosphere, after.atmosphere)) {
    parts.push(after.atmosphere.lights.length !== before.atmosphere.lights.length ? "Light added / removed" : "Atmosphere & light");
  }
  if (!same(before.curves.sCurve, after.curves.sCurve)) parts.push(after.curves.sCurve.enabled ? "S-curve" : "S-curve off");
  else if (!same(before.curves, after.curves)) parts.push("Curves");
  if (!same(before.splitToning, after.splitToning)) parts.push("Split toning");
  if (!same(before.finishing, after.finishing)) parts.push("Grain & vignette");
  if (!same(before.style, after.style)) {
    const s = after.style;
    if (s.blend.length > 0) parts.push("Style fusion");
    else if (!s.id) parts.push("Style removed");
    else if (s.id !== before.style.id) parts.push(`Style: ${styleName(s.id)}`);
    else parts.push(`${styleName(s.id)} ${Math.round(s.amount)}%`);
  }
  if (!same(before.lut, after.lut)) parts.push(after.lut.name ? `LUT: ${after.lut.name}` : "LUT removed");
  if (!same(before.demosaic, after.demosaic)) parts.push("Demosaic");

  if (parts.length === 0) return "Edit";
  if (parts.length > 3) return `${parts.slice(0, 2).join(", ")} + ${parts.length - 2} more`;
  return parts.join(", ");
}
