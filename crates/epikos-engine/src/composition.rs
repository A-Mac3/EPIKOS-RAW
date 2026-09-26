//! Subject ranking and people-aware crop rules for the AI Mentor.
//!
//! **Ranking.** The subject mask can hold several subjects; each connected region is
//! ranked by its share of the frame and how near it is (depth map), so the primary
//! subject is the large, near one and the rest are secondary.
//!
//! **Crops of people** follow the rules portrait photographers work to, not geometry:
//! - never cut through a joint: the neck, the waist / elbows, the knees or the
//!   ankles and feet;
//! - either **full body**, with 5–8 % of the crop's height clear below the feet, or a
//!   clean **three-quarter** crop at mid-thigh, above the knees;
//! - headroom above the head (4–12 % of the crop), never a cropped crown;
//! - lead room in the direction the person faces.
//!
//! There is no pose model, so the body is read from the subject silhouette: the head
//! from skin in the top of the silhouette, body height from the feet when they're in
//! frame, else from the head (≈ ⅛ of body height), and joints at their usual share
//! of body height. Anatomy and impact come before exact thirds: a crop that would cut
//! a joint is never offered, however well it would align.

use serde::Serialize;

use crate::guidance::CropAdvice;
use epikos_sidecar::Crop;

/// A subject region, ranked.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedSubject {
    /// "primary" or "secondary".
    pub rank: &'static str,
    /// Share of the frame, 0–1.
    pub area: f32,
    /// 0 (far) … 1 (near), when a depth map is available.
    pub nearness: Option<f32>,
    /// Left, top, right, bottom (2–98 % extent), fractions of the frame.
    pub bbox: [f32; 4],
    /// Whether it reads as a person (skin in it).
    pub person: bool,
    #[serde(skip)]
    pixels: Vec<usize>,
}

/// Connected subject regions (mask ≥ 0.5, at least 0.3 % of the frame), ranked by
/// area and nearness.
pub(crate) fn rank_subjects(subject: &[f32], skin: Option<&[f32]>, depth: Option<&[f32]>, w: usize, h: usize) -> Vec<RankedSubject> {
    let n = w * h;
    let mut label = vec![u32::MAX; n];
    let mut regions: Vec<Vec<usize>> = Vec::new();
    for start in 0..n {
        if subject[start] < 0.5 || label[start] != u32::MAX {
            continue;
        }
        let id = regions.len() as u32;
        let mut stack = vec![start];
        let mut pixels = Vec::new();
        label[start] = id;
        while let Some(i) = stack.pop() {
            pixels.push(i);
            let (x, y) = (i % w, i / w);
            let mut visit = |j: usize| {
                if subject[j] >= 0.5 && label[j] == u32::MAX {
                    label[j] = id;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - w);
            }
            if y + 1 < h {
                visit(i + w);
            }
        }
        regions.push(pixels);
    }
    let mut out: Vec<(f32, RankedSubject)> = regions
        .into_iter()
        .filter(|p| p.len() as f32 / n as f32 >= 0.003)
        .map(|pixels| {
            let area = pixels.len() as f32 / n as f32;
            let nearness = depth.map(|d| pixels.iter().map(|&i| d[i]).sum::<f32>() / pixels.len() as f32);
            let skin_share = skin.map_or(0.0, |s| pixels.iter().filter(|&&i| s[i] > 0.5).count() as f32 / pixels.len() as f32);
            let mut xs: Vec<f32> = pixels.iter().map(|&i| (i % w) as f32 / w as f32).collect();
            let mut ys: Vec<f32> = pixels.iter().map(|&i| (i / w) as f32 / h as f32).collect();
            xs.sort_unstable_by(f32::total_cmp);
            ys.sort_unstable_by(f32::total_cmp);
            let q = |v: &[f32], p: f32| v[((v.len() - 1) as f32 * p) as usize];
            let bbox = [q(&xs, 0.02), q(&ys, 0.0), q(&xs, 0.98), q(&ys, 1.0)];
            let score = area * (0.6 + 0.8 * nearness.unwrap_or(0.5));
            (score, RankedSubject { rank: "secondary", area, nearness, bbox, person: skin_share > 0.01, pixels })
        })
        .collect();
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut ranked: Vec<RankedSubject> = out.into_iter().map(|(_, s)| s).collect();
    if let Some(first) = ranked.first_mut() {
        first.rank = "primary";
    }
    ranked
}

/// The body of a person, read from their silhouette (fractions of the frame).
#[derive(Debug, Clone, Copy)]
struct Body {
    top: f32,
    left: f32,
    right: f32,
    /// Estimated full height (the feet may be out of frame).
    height: f32,
    feet_in_frame: bool,
    feet: f32,
    /// −1 facing left … +1 facing right, 0 straight on.
    facing: f32,
    center_x: f32,
}

/// Joint positions as a share of body height from the top of the head: cutting a
/// frame edge in these bands reads as an amputation.
const JOINTS: [(f32, f32, &str); 4] = [
    (0.10, 0.20, "the neck"),
    (0.44, 0.56, "the waist and elbows"),
    (0.68, 0.79, "the knees"),
    (0.88, 1.01, "the ankles and feet"),
];
/// A three-quarter crop ends at mid-thigh.
const THREE_QUARTER: f32 = 0.62;
/// Clear ground below the feet in a full-body crop, as a share of the crop height.
const FEET_CLEARANCE: (f32, f32) = (0.05, 0.08);
const HEADROOM: (f32, f32) = (0.04, 0.12);

fn body_of(s: &RankedSubject, skin: Option<&[f32]>, w: usize, h: usize) -> Body {
    let [left, top, right, bottom] = s.bbox;
    let feet_in_frame = bottom < 0.985;
    // Head: skin in the top part of the silhouette.
    let head: Vec<usize> = skin.map_or(Vec::new(), |sk| {
        s.pixels
            .iter()
            .copied()
            .filter(|&i| sk[i] > 0.5 && ((i / w) as f32 / h as f32) < top + 0.35 * (bottom - top))
            .collect()
    });
    let head_h = head.iter().map(|&i| (i / w) as f32 / h as f32).fold(top, f32::max) - top;
    let height = if feet_in_frame { bottom - top } else { (bottom - top).max(8.0 * head_h.max(0.02)) };
    let center_x = s.pixels.iter().map(|&i| (i % w) as f32 / w as f32).sum::<f32>() / s.pixels.len().max(1) as f32;
    let face_x = if head.is_empty() {
        center_x
    } else {
        head.iter().map(|&i| (i % w) as f32 / w as f32).sum::<f32>() / head.len() as f32
    };
    let facing = ((face_x - center_x) / (right - left).max(0.02) * 6.0).clamp(-1.0, 1.0);
    Body { top, left, right, height, feet_in_frame, feet: bottom, facing, center_x }
}

/// Which joint a horizontal edge at `y` (frame fraction) would cut, if any.
fn cut_joint(b: &Body, y: f32) -> Option<&'static str> {
    let f = (y - b.top) / b.height;
    JOINTS.iter().find(|(lo, hi, _)| f > *lo && f < *hi).map(|j| j.2)
}

/// Whether the crop (x, y, s×s) keeps the person whole where it must, and every edge
/// clear of joints.
fn anatomy_ok(b: &Body, [x, y, s]: [f32; 3]) -> bool {
    let bottom = y + s;
    let head_room = (b.top - y) / s;
    let sides = b.left >= x + 0.01 * s && b.right <= x + s - 0.01 * s;
    let bottom_ok = if bottom >= b.feet {
        // Full body: 5–8 % clear below the feet, or the frame's own bottom edge.
        let clear = (bottom - b.feet) / s;
        clear >= FEET_CLEARANCE.0 - 1e-3 && (clear <= FEET_CLEARANCE.1 + 1e-3 || bottom >= 0.999)
    } else {
        cut_joint(b, bottom).is_none()
    };
    sides && bottom_ok && (HEADROOM.0 - 1e-3..=HEADROOM.1 + 1e-3).contains(&head_room)
}

/// A crop of the frame's own aspect framing the primary person full-body or
/// three-quarter, with headroom and lead room; with the reason, or `None` when the
/// frame already follows the rules or no rule-abiding crop exists.
pub(crate) fn person_crop(primary: &RankedSubject, skin: Option<&[f32]>, w: usize, h: usize, rotation: f32) -> Option<CropAdvice> {
    if !primary.person {
        return None;
    }
    let b = body_of(primary, skin, w, h);
    // What's wrong with the frame as it is.
    let mut problems = Vec::new();
    if !b.feet_in_frame {
        if let Some(j) = cut_joint(&b, 1.0) {
            problems.push(format!("the bottom edge cuts {j}"));
        }
    } else if (1.0 - b.feet) < 0.02 {
        problems.push("the feet touch the bottom edge".to_string());
    }
    if b.top < 0.02 {
        problems.push("the head touches the top edge".to_string());
    } else if b.top > 0.2 && b.height < 0.6 {
        problems.push("there's a lot of empty space above the head".to_string());
    }
    let lead = if b.facing.abs() > 0.25 {
        // Space in front of the face: subject on the third away from where they look.
        let crowded = if b.facing < 0.0 { b.center_x < 0.4 } else { b.center_x > 0.6 };
        if crowded {
            problems.push(format!("they face the {} edge with little room ahead", if b.facing < 0.0 { "left" } else { "right" }));
        }
        Some(if b.facing < 0.0 { 2.0 / 3.0 } else { 1.0 / 3.0 })
    } else {
        None
    };
    if problems.is_empty() {
        return None;
    }

    // Candidate framings: full body (if the feet are in frame) and three-quarter.
    let mut candidates: Vec<([f32; 3], &str)> = Vec::new();
    for headroom in [0.06f32, 0.08, 0.1] {
        if b.feet_in_frame {
            let clear = 0.065;
            let s = (b.feet - b.top) / (1.0 - headroom - clear);
            candidates.push(([0.0, b.top - headroom * s, s], "full body"));
        }
        let bottom = b.top + THREE_QUARTER * b.height;
        if bottom < b.feet - 0.02 {
            let s = (bottom - b.top) / (1.0 - headroom);
            candidates.push(([0.0, b.top - headroom * s, s], "three-quarter, at mid-thigh"));
        }
    }
    let mut best: Option<(f32, [f32; 3], &str)> = None;
    for ([_, y, s], kind) in candidates {
        if !(0.3..=1.0).contains(&s) || y < -1e-3 || y + s > 1.0 + 1e-3 {
            continue;
        }
        // Horizontal: lead room if they face a side, else keep their place in frame.
        let u = lead.unwrap_or_else(|| b.center_x.clamp(1.0 / 3.0, 2.0 / 3.0));
        let x = (b.center_x - u * s).clamp(0.0, 1.0 - s);
        let r = [x, y.clamp(0.0, 1.0 - s), s];
        if !anatomy_ok(&b, r) {
            continue;
        }
        // Prefer keeping more of the frame, then full body.
        let score = s + if kind == "full body" { 0.05 } else { 0.0 };
        if best.is_none_or(|(sc, _, _)| score > sc) {
            best = Some((score, r, kind));
        }
    }
    let (_, [x, y, s], kind) = best?;
    if s > 0.985 && x < 0.01 && y < 0.01 {
        return None;
    }
    let mut reason = format!("frames the main person {kind} (because {})", problems.join(" and "));
    reason += ", with headroom";
    if b.feet_in_frame && kind == "full body" {
        reason += " and 5–8% of ground below the feet";
    }
    if lead.is_some() {
        reason += " and room in the direction they face";
    }
    Some(CropAdvice { crop: Crop { x, y, width: s, height: s, aspect: "original".into() }, rotation, reason })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A standing person: head (skin) at the top, body below, in a w×h frame.
    fn person(w: usize, h: usize, x0: usize, x1: usize, top: usize, bottom: usize) -> (Vec<f32>, Vec<f32>) {
        let mut subject = vec![0.0; w * h];
        let mut skin = vec![0.0; w * h];
        for y in top..bottom.min(h) {
            for x in x0..x1 {
                subject[y * w + x] = 1.0;
                if y < top + (bottom - top) / 8 {
                    skin[y * w + x] = 1.0;
                }
            }
        }
        (subject, skin)
    }

    #[test]
    fn subjects_are_ranked_by_area_and_nearness() {
        let (w, h) = (100, 60);
        let mut subject = vec![0.0; w * h];
        let mut depth = vec![0.0; w * h];
        for y in 10..50 {
            for x in 5..20 {
                subject[y * w + x] = 1.0; // larger, far
                depth[y * w + x] = 0.1;
            }
            for x in 60..72 {
                subject[y * w + x] = 1.0; // smaller, near
                depth[y * w + x] = 1.0;
            }
        }
        let r = rank_subjects(&subject, None, Some(&depth), w, h);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].rank, "primary");
        assert!(r[0].bbox[0] > 0.5, "the near subject leads: {:?}", r[0].bbox);
        assert_eq!(r[1].rank, "secondary");
    }

    #[test]
    fn a_crop_never_cuts_a_joint_and_keeps_feet_clearance() {
        let (w, h) = (90, 60);
        // Feet almost on the bottom edge and head touching the top.
        let (subject, skin) = person(w, h, 40, 50, 0, 59);
        let r = rank_subjects(&subject, Some(&skin), None, w, h);
        let advice = person_crop(&r[0], Some(&skin), w, h, 0.0);
        // With the head at the very top there's no room for headroom inside the
        // frame and a crop can't add it: nothing rule-abiding is offered.
        assert!(advice.is_none(), "{advice:?}");

        // A small figure with a lot of empty space above: a crop brings them forward.
        let (subject, skin) = person(w, h, 20, 26, 20, 50);
        let r = rank_subjects(&subject, Some(&skin), None, w, h);
        let advice = person_crop(&r[0], Some(&skin), w, h, 0.0).expect("a crop");
        let c = &advice.crop;
        let b = body_of(&r[0], Some(&skin), w, h);
        let bottom = c.y + c.height;
        if bottom >= b.feet {
            let clear = (bottom - b.feet) / c.height;
            assert!((0.049..=0.081).contains(&clear) || bottom >= 0.999, "clearance {clear}");
        } else {
            assert!(cut_joint(&b, bottom).is_none(), "cut at {}", (bottom - b.top) / b.height);
        }
        let headroom = (b.top - c.y) / c.height;
        assert!((0.039..=0.121).contains(&headroom), "headroom {headroom}");
        assert!(advice.reason.contains("main person"), "{}", advice.reason);
    }

    #[test]
    fn joints_are_where_photographers_expect_them() {
        let b = Body { top: 0.0, left: 0.4, right: 0.6, height: 1.0, feet_in_frame: true, feet: 1.0, facing: 0.0, center_x: 0.5 };
        assert_eq!(cut_joint(&b, 0.74), Some("the knees"));
        assert_eq!(cut_joint(&b, 0.95), Some("the ankles and feet"));
        assert_eq!(cut_joint(&b, THREE_QUARTER), None, "mid-thigh is a clean cut");
        assert_eq!(cut_joint(&b, 0.3), None, "mid-chest is a clean cut");
    }
}
