//! the Rocketbox clips and the 100STYLE walks on the Rocketbox
//! people. The checks that read the copies (in `<repo>/external/people`,
//! `people-motions` and `people-styles`, or the folders `EO_PEOPLE_ROOT`,
//! `EO_PEOPLE_MOTIONS_ROOT` and `EO_PEOPLE_STYLES_ROOT` name) are skipped
//! without them. `-- --nocapture` prints what was measured.

use eo_agents::motion::{self, Family, Manifest, ROLES, STYLES};
use eo_agents::rocketbox::{self, BakeSrc, BipSkeleton, JOINTS};
use glam::{Vec3, Vec4};
use std::path::PathBuf;

struct Copies {
    people: PathBuf,
    clips: PathBuf,
    styles: PathBuf,
}

fn copies() -> Option<Copies> {
    let people = rocketbox::root_from_env();
    let m = motion::MotionRoots::from_env();
    match (people, m.clips, m.styles) {
        (Some(people), Some(clips), Some(styles)) => Some(Copies { people, clips, styles }),
        _ => {
            eprintln!("people-motion: a copy is missing (people, clips or styles); skipped");
            None
        }
    }
}

/// One avatar per distinct Bip01 skeleton among the pool.
fn groups(root: &std::path::Path) -> Vec<(String, BipSkeleton)> {
    let m = rocketbox::Manifest::load().unwrap();
    let mut out: Vec<(String, BipSkeleton)> = vec![];
    for a in rocketbox::pool(&m, 200) {
        let sk = rocketbox::skeleton_only(root, &m, a).unwrap();
        if !out.iter().any(|(_, s)| s.key() == sk.key()) {
            out.push((a.id.clone(), sk));
        }
    }
    out
}

fn idx(name: &str) -> usize {
    JOINTS.iter().position(|(b, _)| *b == name).unwrap_or_else(|| panic!("no joint {name}"))
}

/// A joint's position at `frame` of a table (its bind position through the frame's matrix).
fn at(rows: &[[f32; 4]], sk: &BipSkeleton, frame: usize, k: usize) -> Vec3 {
    let n = JOINTS.len();
    let p = sk.pos[k].extend(1.0);
    let r = &rows[(frame * n + k) * 3..(frame * n + k) * 3 + 3];
    Vec3::new(Vec4::from(r[0]).dot(p), Vec4::from(r[1]).dot(p), Vec4::from(r[2]).dot(p))
}

fn median(mut v: Vec<f32>) -> f32 {
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

fn elbow_bend(rows: &[[f32; 4]], sk: &BipSkeleton, f: usize, side: &str) -> f32 {
    let (u, e, h) = (at(rows, sk, f, idx(&format!("Bip01 {side} UpperArm"))), at(rows, sk, f, idx(&format!("Bip01 {side} Forearm"))), at(rows, sk, f, idx(&format!("Bip01 {side} Hand"))));
    (e - u).normalize().angle_between((h - e).normalize()).to_degrees()
}

/// A point fixed to joint `k` (`offset` from its bind position, bind frame) at `frame`.
fn at_point(rows: &[[f32; 4]], k: usize, frame: usize, p: Vec3) -> Vec3 {
    let n = JOINTS.len();
    let p = p.extend(1.0);
    let r = &rows[(frame * n + k) * 3..(frame * n + k) * 3 + 3];
    Vec3::new(Vec4::from(r[0]).dot(p), Vec4::from(r[1]).dot(p), Vec4::from(r[2]).dot(p))
}

/// The closer hand's knuckles to the nearer ear (7.5 cm to the side of and
/// 5 cm above the head joint at bind, moving with the head), metres.
fn hand_to_head(rows: &[[f32; 4]], sk: &BipSkeleton, f: usize) -> f32 {
    let h = idx("Bip01 Head");
    let ears = [at_point(rows, h, f, sk.pos[h] + Vec3::new(0.075, 0.05, 0.0)), at_point(rows, h, f, sk.pos[h] + Vec3::new(-0.075, 0.05, 0.0))];
    ["L", "R"].iter().flat_map(|s| { let k = at(rows, sk, f, idx(&format!("Bip01 {s} Finger2"))); ears.map(|e| (k - e).length()) }).fold(f32::MAX, f32::min)
}

#[test]
fn manifests_pin_the_sources() {
    let c = Manifest::clips().unwrap();
    assert_eq!(c.files.iter().filter(|f| f.path.starts_with("objects/")).count(), 326, "every Rocketbox in-place clip is listed");
    for r in ROLES {
        for id in r.clips() {
            assert!(c.file(&motion::clip_file(id)).is_some(), "{id} has no digest");
        }
        assert!(r.fps * r.seconds >= 60.0 && r.slot >= 1 && r.slot <= 3, "{}", r.name);
    }
    for f in [Family::Idle, Family::Wait, Family::Talk, Family::Listen, Family::Call, Family::Text, Family::Wave, Family::Point, Family::Sit, Family::Photo] {
        assert!(ROLES.iter().any(|r| r.family == f), "no loop for {f:?}");
    }
    let s = Manifest::styles().unwrap();
    for (style, _) in STYLES {
        assert!(s.file(&motion::style_file(style)).is_some(), "{style} has no digest");
    }
    assert!(c.file("LICENSES.md").is_some() && s.file("LICENSES.md").is_some());
}

#[test]
fn styles_cut_clean_cycles() {
    let Some(cp) = copies() else { return };
    let m = Manifest::styles().unwrap();
    let mut left_out = vec![];
    for (style, phone) in STYLES {
        let c = match motion::load_style(&cp.styles, &m, style, phone) {
            Ok(c) => c,
            Err(e) => {
                println!("{style:<16} left out: {e:#}");
                left_out.push(style);
                continue;
            }
        };
        println!("{style:<16} cycle {:.2} s from {:6.1} s, {:.2} m, {:.2} m/s, {} cycles in the take, turn {:.1} deg, stride x {:.2}", c.seconds, c.from_s, c.stride, c.speed, c.cycles, c.turn_deg, c.stride_scale);
        assert_eq!(c.frames.len(), motion::WALK_FRAMES as usize + 1);
        assert_eq!(c.frames[0].0, c.frames[motion::WALK_FRAMES as usize].0, "{style}: the loop is closed");
        assert!((0.6..1.7).contains(&c.seconds) && (0.5..2.2).contains(&c.stride) && (0.3..2.2).contains(&c.speed), "{style}: {:.2} s, {:.2} m", c.seconds, c.stride);
        // In place: the hips' offset stays near the cycle's line.
        for (_, off) in &c.frames {
            assert!(off.x.abs() < 0.2 && off.z.abs() < 0.2, "{style}: hips {off}");
        }
    }
    assert!(left_out.len() <= 3, "styles with no clean cycle: {left_out:?}");
}

/// Every role and style on each proportion group: standing on the ground,
/// relaxed idle arms, the phone at the ear, walks that plant, seamless loops.
#[test]
fn clips_and_walks_on_every_group() {
    let Some(cp) = copies() else { return };
    let cm = Manifest::clips().unwrap();
    let sm = Manifest::styles().unwrap();
    let mut ids: Vec<&str> = ROLES.iter().flat_map(|r| r.clips()).collect();
    ids.sort_unstable();
    ids.dedup();
    let clips: Vec<motion::RbClip> = ids.iter().map(|id| motion::load_rb_clip(&cp.clips, &cm, id).unwrap_or_else(|e| panic!("{id}: {e:#}"))).collect();
    let clip = |id: &str| clips.iter().find(|c| c.id == id).unwrap();
    let mut styles: Vec<motion::StyleCycle> = STYLES.iter().filter_map(|(s, p)| motion::load_style(&cp.styles, &sm, s, *p).ok()).collect();
    motion::level_heads(&mut styles);
    let gs = groups(&cp.people);
    println!("{} proportion groups: {}", gs.len(), gs.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>().join(", "));
    assert!(gs.len() >= 2);
    for (id, sk) in &gs {
        let female = id.contains("female");
        let relaxed = rocketbox::relaxed_hands(sk, clip(if female { ROLES[0].female } else { ROLES[0].male }), &ROLES[0]).unwrap();
        let mut srcs: Vec<BakeSrc> = styles.iter().map(|c| BakeSrc::Style { cycle: c }).collect();
        let role_frames: Vec<u32> = ROLES.iter().map(|r| (r.seconds * r.fps).round() as u32 + 1).collect();
        for (r, role) in ROLES.iter().enumerate() {
            srcs.push(BakeSrc::Clip { clip: clip(if female { role.female } else { role.male }), role, frames: role_frames[r], layer: role.layer(female).map(|(id, _)| clip(id)) });
        }
        let b = rocketbox::bake_list(sk, &srcs, Some(&relaxed)).unwrap();
        assert!(b.rows.iter().all(|r| r.iter().all(|x| x.is_finite())));
        // Frame ranges of each stretch.
        let mut first = vec![];
        let mut f = 0;
        let mut lens = vec![];
        for s in &srcs {
            let n = match s {
                BakeSrc::Style { .. } => motion::WALK_FRAMES as usize + 1,
                BakeSrc::Clip { frames, .. } => *frames as usize,
            };
            first.push(f);
            lens.push(n);
            f += n;
        }
        assert_eq!(b.rows.len(), f * JOINTS.len() * 3);
        let pelvis = idx("Bip01 Pelvis");
        println!("== {id} ({}), pelvis {:.3} m", if female { "women" } else { "men" }, sk.pos[pelvis].y);
        for (i, s) in srcs.iter().enumerate() {
            assert!(b.lifts[i].abs() < 0.15, "{id} stretch {i}: lift {}", b.lifts[i]);
            let (a, n) = (first[i], lens[i]);
            let frames = a..a + n;
            match s {
                BakeSrc::Style { cycle } => {
                    let (stride, slide) = b.strides[i].unwrap_or_else(|| panic!("{id} {}: no planted foot", cycle.name));
                    let scaled = cycle.stride * cycle.stride_scale * sk.pos[pelvis].y / cycle.nodes.iter().find(|n| n.name == "Hips").unwrap().t.y;
                    println!("  walk {:<16} stride {stride:.3} m (by leg length {scaled:.3}), natural {:.2} m/s, median slide {:.1} % (lift {:+.3})", cycle.name, stride / cycle.seconds, slide * 100.0, b.lifts[i]);
                    assert!((stride / scaled - 1.0).abs() < 0.3, "{id} {}: stride {stride} against {scaled}", cycle.name);
                    assert!(slide < 0.3, "{id} {}: slide {slide}", cycle.name);
                }
                BakeSrc::Clip { role, .. } => {
                    let name = role.name;
                    let bend: Vec<f32> = frames.clone().map(|f| 0.5 * (elbow_bend(&b.rows, sk, f, "L") + elbow_bend(&b.rows, sk, f, "R"))).collect();
                    let ear: Vec<f32> = frames.clone().map(|f| hand_to_head(&b.rows, sk, f)).collect();
                    let nearest = ear.iter().enumerate().min_by(|x, y| x.1.total_cmp(y.1)).map(|(k, _)| k as f32 / n as f32).unwrap_or(0.0);
                    let hand_y: Vec<f32> = frames.clone().map(|f| at(&b.rows, sk, f, idx("Bip01 L Hand")).y.max(at(&b.rows, sk, f, idx("Bip01 R Hand")).y) - at(&b.rows, sk, f, pelvis).y).collect();
                    let hand_x: Vec<f32> = frames.clone().map(|f| (at(&b.rows, sk, f, idx("Bip01 L Hand")) - at(&b.rows, sk, f, idx("Bip01 L Thigh"))).x).collect();
                    let hand_z: Vec<f32> = frames.clone().map(|f| (at(&b.rows, sk, f, idx("Bip01 L Hand")) - at(&b.rows, sk, f, pelvis)).z).collect();
                    println!(
                        "  {name:<11} elbows {:4.1} deg, knuckles to ear median {:.3} m (min {:.3}, max {:.3}), higher hand {:+.3} m over the pelvis, left hand {:+.3} m out of the thigh, {:+.3} m ahead (lift {:+.3}), nearest the ear at phase {nearest:.2}",
                        median(bend.clone()), median(ear.clone()), ear.iter().cloned().fold(f32::MAX, f32::min), ear.iter().cloned().fold(0.0, f32::max), median(hand_y.clone()), median(hand_x), median(hand_z), b.lifts[i]
                    );
                    if let BakeSrc::Clip { role, .. } = s {
                        if role.family == Family::Idle {
                            assert!(median(bend) > 8.0, "{id} {}: the elbows hang straight", role.name);
                            assert!(median(hand_y) < 0.0, "{id} {}: a hand above the pelvis", role.name);
                        }
                        if role.family == Family::Call {
                            assert!(median(ear.clone()) < 0.12, "{id} {}: the knuckles are {:.3} m from the ear", role.name, median(ear));
                        }
                        // The players' emotes hold the pose they are named for over most of the loop.
                        let share = |ok: &dyn Fn(usize) -> bool| frames.clone().filter(|f| ok(*f)).count() as f32 / n as f32;
                        let head = |f: usize| at(&b.rows, sk, f, idx("Bip01 Head"));
                        let hand = |f: usize, s: &str| at(&b.rows, sk, f, idx(&format!("Bip01 {s} Hand")));
                        let pel = |f: usize| at(&b.rows, sk, f, pelvis);
                        let reach = |f: usize, s: &str| (hand(f, s) - pel(f)).x.hypot((hand(f, s) - pel(f)).z);
                        let eyes = |f: usize| (at(&b.rows, sk, f, idx("Bip01 LEye")) + at(&b.rows, sk, f, idx("Bip01 REye"))) * 0.5;
                        // The index finger's bend (degrees between its first and last segments) against the middle finger's.
                        let bend3 = |f: usize, n: &str| {
                            let (a0, a1, a2) = (at(&b.rows, sk, f, idx(&format!("Bip01 R Finger{n}"))), at(&b.rows, sk, f, idx(&format!("Bip01 R Finger{n}1"))), at(&b.rows, sk, f, idx(&format!("Bip01 R Finger{n}2"))));
                            let h = at(&b.rows, sk, f, idx("Bip01 R Hand"));
                            (a0 - h).normalize().angle_between((a2 - a1).normalize()).to_degrees()
                        };
                        let held = match role.family {
                            // One hand up (the right, above the head), the other down by the hip, the forearm swinging.
                            Family::Wave => {
                                let lateral: Vec<f32> = frames.clone().map(|f| (hand(f, "R") - head(f)).x).collect();
                                let swing = lateral.iter().cloned().fold(f32::MIN, f32::max) - lateral.iter().cloned().fold(f32::MAX, f32::min);
                                println!("  {name:<11} the waving hand swings {swing:.2} m across");
                                assert!(swing > 0.12, "{id} wave: the hand swings only {swing:.2} m");
                                Some(("the right hand above the head, the left below the pelvis", share(&|f| hand(f, "R").y > head(f).y && hand(f, "L").y < pel(f).y)))
                            }
                            Family::Point => {
                                let (ix, mid) = (median(frames.clone().map(|f| bend3(f, "1")).collect()), median(frames.clone().map(|f| bend3(f, "2")).collect()));
                                println!("  {name:<11} index bent {ix:.0} deg, middle {mid:.0} deg");
                                assert!(ix < 30.0 && mid > ix + 60.0, "{id} point: index {ix:.0} deg, middle {mid:.0} deg");
                                Some(("an arm held out 0.5 m at shoulder height", share(&|f| ["L", "R"].iter().any(|s| reach(f, s) > 0.5 && hand(f, s).y > head(f).y - 0.35))))
                            }
                            // The selfie: the right hand 0.3 to 0.6 m from the eyes, out in front, within 0.2 m of their height, the left down.
                            Family::Photo => Some(("the phone hand held out before the face", {
                                let d: Vec<Vec3> = frames.clone().map(|f| hand(f, "R") - eyes(f)).collect();
                                println!("  {name:<11} the phone hand from the eyes: {:+.2} right, {:+.2} up, {:+.2} ahead (median)", -median(d.iter().map(|v| v.x).collect()), median(d.iter().map(|v| v.y).collect()), median(d.iter().map(|v| v.z).collect()));
                                share(&|f| {
                                let d = hand(f, "R") - eyes(f);
                                (0.3..0.6).contains(&d.length()) && d.z > 0.15 && d.y.abs() < 0.2 && hand(f, "L").y < pel(f).y
                            })
                            })),
                            Family::Sit => Some(("the pelvis 0.2 m under standing", share(&|f| pel(f).y < sk.pos[pelvis].y - 0.2))),
                            _ => None,
                        };
                        if let Some((what, k)) = held {
                            println!("  {name:<11} {what}: {:.0} % of the loop", k * 100.0);
                            assert!(k >= 0.6, "{id} {}: {what} in only {:.0} % of the loop", role.name, k * 100.0);
                        }
                        // The seam: no bigger a step than the loop's own.
                        let step = |f: usize| -> f32 { (0..JOINTS.len()).map(|k| (at(&b.rows, sk, f + 1, k) - at(&b.rows, sk, f, k)).length()).fold(0.0, f32::max) };
                        let steps: Vec<f32> = (a..a + n - 1).map(step).collect();
                        let seam = steps[steps.len() - 1].max(steps[0]);
                        let mut sorted = steps.clone();
                        sorted.sort_by(f32::total_cmp);
                        // The seam is a step of the clip itself (the window's end into the cross-faded start).
                        let max = sorted[sorted.len() - 1];
                        assert!(seam <= 1.0001 * max && seam < 0.25, "{id} {}: seam step {seam} against the loop's largest {max}", role.name);
                    }
                }
            }
        }
    }
}

/// The pool's exact skeleton keys split the men's one skeleton three ways;
/// with the motions in they share one table (`rocketbox::same_bind`).
#[test]
fn proportion_groups_with_the_motions() {
    let Some(cp) = copies() else { return };
    let gs = groups(&cp.people);
    let mut merged: Vec<&BipSkeleton> = vec![];
    for (i, (id, sk)) in gs.iter().enumerate() {
        for (jd, other) in &gs[i + 1..] {
            let dp = (0..JOINTS.len()).map(|k| (sk.pos[k] - other.pos[k]).length()).fold(0.0f32, f32::max);
            let dr = (0..JOINTS.len()).map(|k| sk.rot[k].angle_between(other.rot[k]).to_degrees()).fold(0.0f32, f32::max);
            println!("{id} vs {jd}: joints up to {dp:.4} m and {dr:.3} deg apart");
        }
        if !merged.iter().any(|m| rocketbox::same_bind(m, sk)) {
            merged.push(sk);
        }
    }
    println!("{} exact keys, {} tables with the motions", gs.len(), merged.len());
    assert_eq!(merged.len(), 2, "one skeleton for the women, one for the men");
}

/// Trunk (pelvis to neck) and head lean of each walk, source and bake,
/// against the idle (degrees, forward positive): the plain walks look ahead
/// once the take's own head pitch is levelled (`motion::level_heads`).
#[test]
fn plain_walks_look_ahead() {
    let Some(cp) = copies() else { return };
    let sm = Manifest::styles().unwrap();
    let cm = Manifest::clips().unwrap();
    let gs = groups(&cp.people);
    let (id, sk) = &gs[0];
    let female = id.contains("female");
    let idle = motion::load_rb_clip(&cp.clips, &cm, if female { ROLES[0].female } else { ROLES[0].male }).unwrap();
    let relaxed = rocketbox::relaxed_hands(sk, &idle, &ROLES[0]).unwrap();
    let pitch = |v: Vec3| v.z.atan2(v.y).to_degrees();
    let (pel, neck, head) = (idx("Bip01 Pelvis"), idx("Bip01 Neck"), idx("Bip01 Head"));
    let eye = idx("Bip01 LEye");
    let bi = rocketbox::bake_list(sk, &[BakeSrc::Clip { clip: &idle, role: &ROLES[0], frames: 101, layer: None }], None).unwrap();
    let lean = |rows: &[[f32; 4]], n: usize| -> (f32, f32) {
        let t: f32 = (0..n).map(|f| pitch(at(rows, sk, f, neck) - at(rows, sk, f, pel))).sum::<f32>() / n as f32;
        let h: f32 = (0..n).map(|f| pitch(at(rows, sk, f, eye) - at(rows, sk, f, head))).sum::<f32>() / n as f32;
        (t, h)
    };
    let (ti, hi) = lean(&bi.rows, 100);
    println!("{id} idle: trunk {ti:+.1}, head-to-eye {hi:+.1}");
    let mut cycles: Vec<motion::StyleCycle> = STYLES.iter().map(|(s, p)| motion::load_style(&cp.styles, &sm, s, *p).unwrap()).collect();
    println!("heads levelled by {:?} degrees", motion::level_heads(&mut cycles));
    for c in &cycles {
        let s = c.name.as_str();
        let si: std::collections::HashMap<&str, usize> = c.nodes.iter().enumerate().map(|(i, n)| (n.name.as_str(), i)).collect();
        let order = eo_agents::motion::topo_nodes(&c.nodes);
        let mut st = 0.0;
        let mut sh = 0.0;
        for (lr, off) in &c.frames[..32] {
            let (wp, _) = eo_agents::motion::fk(&c.nodes, &order, lr, *off);
            st += pitch(wp[si["Neck"]] - wp[si["Hips"]]);
            sh += pitch(wp[si["Head_end"]] - wp[si["Head"]]);
        }
        let b = rocketbox::bake_list(sk, &[BakeSrc::Style { cycle: c }], Some(&relaxed)).unwrap();
        let (bt, bh) = lean(&b.rows, 32);
        println!("{s:<16} source trunk {:+.1} head {:+.1} | bake trunk {bt:+.1} ({:+.1} on the idle), head-to-eye {bh:+.1} ({:+.1})", st / 32.0, sh / 32.0, bt - ti, bh - hi);
        if ["Neutral", "HandsInPockets", "Proud", "Followed"].contains(&s) {
            assert!((bh - hi).abs() < 10.0 && (bt - ti).abs() < 6.0, "{s}: head {:+.1}, trunk {:+.1} degrees off the idle", bh - hi, bt - ti);
        }
    }
}
