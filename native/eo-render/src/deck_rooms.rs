//! The landmark decks' rooms for the camera meter (eo-landmarks access.rs
//! `Interior`; `--no-landmark-access` leaves none). A camera standing inside
//! one (the Coit gallery, the Salesforce observation level, the
//! Transamerica room) meters the room's own surfaces wherever its view does
//! not pass one of the room's openings (eo-sky `night::meter_room`), so the
//! exposure opens up indoors as a camera's reflected-light meter does, and
//! a view out of a window keeps the exposure the city outside asks for.
//! Set once when the landmarks are appended (landmarks.rs `append`).

use std::sync::RwLock;

/// One room: its plan ring and height range (the eye inside), its openings
/// (planar quads), and its interior surfaces' mean sky visibility.
#[derive(Clone, Debug, PartialEq)]
pub struct Interior {
    pub ring: Vec<[f64; 2]>,
    pub y0: f64,
    pub y1: f64,
    pub portals: Vec<[[f64; 3]; 4]>,
    pub sky: f64,
}

static ROOMS: RwLock<Vec<Interior>> = RwLock::new(Vec::new());

/// The rooms in force (replaces any before).
pub fn set(rooms: Vec<Interior>) {
    *ROOMS.write().unwrap_or_else(|e| e.into_inner()) = rooms;
}

/// The rooms of a landmark set's decks (none with access off).
pub fn set_from(access: Option<&eo_landmarks::access::AccessSet>) {
    let rooms = access
        .map(|a| {
            a.decks
                .iter()
                .filter_map(|d| d.interior.as_ref())
                .map(|i| Interior { ring: i.ring.clone(), y0: i.y0, y1: i.y1, portals: i.portals.clone(), sky: i.sky })
                .collect()
        })
        .unwrap_or_default();
    set(rooms);
}

fn inside_ring(ring: &[[f64; 2]], x: f64, z: f64) -> bool {
    let mut inside = false;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a[1] > z) != (b[1] > z) && x < a[0] + (z - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            inside = !inside;
        }
    }
    inside
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// An eye standing in an opening may be a little past its plane (a round
/// wall's arch: its chord lies inside the wall's face), metres.
const IN_OPENING_M: f64 = 0.3;

/// Whether the ray from `o` along `d` (unit) leaves the room through the
/// planar quad `q` (the room's middle at `mid`): heading out of it, and
/// crossing it ahead of `o` or at most `IN_OPENING_M` behind it.
fn crosses(o: [f64; 3], d: [f64; 3], q: &[[f64; 3]; 4], mid: [f64; 3]) -> bool {
    let mut n = cross(sub(q[1], q[0]), sub(q[3], q[0]));
    let qc = [(q[0][0] + q[2][0]) / 2.0, (q[0][1] + q[2][1]) / 2.0, (q[0][2] + q[2][2]) / 2.0];
    if dot(n, sub(qc, mid)) < 0.0 {
        n = [-n[0], -n[1], -n[2]];
    }
    let den = dot(n, d);
    if den <= 1e-12 {
        return false; // along it or back into the room
    }
    let t = dot(n, sub(q[0], o)) / den;
    if t <= -IN_OPENING_M {
        return false;
    }
    let p = [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t];
    // inside the quad: on the same side of all four edges (convex, either winding).
    let mut sign = 0.0;
    for i in 0..4 {
        let s = dot(cross(sub(q[(i + 1) % 4], q[i]), sub(p, q[i])), n);
        if s.abs() < 1e-12 {
            continue;
        }
        if sign == 0.0 {
            sign = s.signum();
        } else if s.signum() != sign {
            return false;
        }
    }
    true
}

/// The player's body for the decks' contact occlusion and room light:
/// (feet, the body's height), metres; None with no player (or access off).
static PLAYER: RwLock<Option<([f64; 3], f64)>> = RwLock::new(None);

/// The player's body this frame (eo-app: the shot's person, the played body).
pub fn set_player(body: Option<([f64; 3], f64)>) {
    *PLAYER.write().unwrap_or_else(|e| e.into_inner()) = body;
}

/// The look uniform's deck terms (look_common.wgsl `deck_room_min`,
/// `deck_room_max`, `deck_player`): the room the player stands in (else the
/// room the camera stands in) as a box with its sky visibility (w < 0: none),
/// and the player's feet and height (w 0: none).
pub fn uniform(eye: [f64; 3]) -> ([f32; 4], [f32; 4], [f32; 4]) {
    let none = ([0.0, 0.0, 0.0, -1.0], [0.0; 4], [0.0; 4]);
    let rooms = ROOMS.read().unwrap_or_else(|e| e.into_inner());
    if rooms.is_empty() {
        return none;
    }
    let player = *PLAYER.read().unwrap_or_else(|e| e.into_inner());
    let at = |p: [f64; 3]| rooms.iter().find(|r| p[1] >= r.y0 - 0.5 && p[1] <= r.y1 && inside_ring(&r.ring, p[0], p[2]));
    let room = player.and_then(|(f, _)| at(f)).or_else(|| at(eye));
    let (lo, hi) = match room {
        Some(r) => {
            let (mut lo, mut hi) = ([f64::MAX, r.y0 - 0.5, f64::MAX], [f64::MIN, r.y1, f64::MIN]);
            for q in &r.ring {
                lo[0] = lo[0].min(q[0]);
                lo[2] = lo[2].min(q[1]);
                hi[0] = hi[0].max(q[0]);
                hi[2] = hi[2].max(q[1]);
            }
            ([lo[0] as f32, lo[1] as f32, lo[2] as f32, r.sky as f32], [hi[0] as f32, hi[1] as f32, hi[2] as f32, 0.0])
        }
        None => (none.0, none.1),
    };
    let body = player.map_or([0.0; 4], |(f, h)| [f[0] as f32, f[1] as f32, f[2] as f32, h as f32]);
    (lo, hi, body)
}

/// For an eye inside one of the rooms: per direction, None where the ray
/// leaves through one of the room's openings, else Some(the room's sky
/// visibility). None when the eye stands in no room.
pub fn classify(eye: [f64; 3], dirs: &[[f64; 3]]) -> Option<Vec<Option<f64>>> {
    let rooms = ROOMS.read().unwrap_or_else(|e| e.into_inner());
    classify_in(&rooms, eye, dirs)
}

fn classify_in(rooms: &[Interior], eye: [f64; 3], dirs: &[[f64; 3]]) -> Option<Vec<Option<f64>>> {
    let room = rooms.iter().find(|r| eye[1] >= r.y0 && eye[1] <= r.y1 && inside_ring(&r.ring, eye[0], eye[2]))?;
    let k = room.ring.len().max(1) as f64;
    let mid = [room.ring.iter().map(|p| p[0]).sum::<f64>() / k, (room.y0 + room.y1) / 2.0, room.ring.iter().map(|p| p[1]).sum::<f64>() / k];
    Some(
        dirs.iter()
            .map(|d| {
                let l = dot(*d, *d).sqrt().max(1e-12);
                let u = [d[0] / l, d[1] / l, d[2] / l];
                if room.portals.iter().any(|q| crosses(eye, u, q, mid)) { None } else { Some(room.sky) }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ray_out_of_the_window_meters_the_outside_and_the_rest_the_room() {
        // a 10 m square room, 3 m high, a window in its east wall.
        let room = Interior {
            ring: vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0]],
            y0: 0.0,
            y1: 3.0,
            portals: vec![[[5.0, 0.5, -2.0], [5.0, 0.5, 2.0], [5.0, 2.5, 2.0], [5.0, 2.5, -2.0]]],
            sky: 0.2,
        };
        let rooms = vec![room];
        let eye = [0.0, 1.6, 0.0];
        let k = classify_in(&rooms, eye, &[[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.7071, 0.0, 0.7071]]).unwrap();
        assert_eq!(k, vec![None, Some(0.2), Some(0.2), Some(0.2)]);
        // outside the room: not metered as one.
        assert!(classify_in(&rooms, [8.0, 1.6, 0.0], &[[1.0, 0.0, 0.0]]).is_none());
        assert!(classify_in(&rooms, [0.0, 4.0, 0.0], &[[1.0, 0.0, 0.0]]).is_none());
        // an eye standing in the opening, a little past its plane (the room's
        // ring reaching past it, as a round wall's face past its arch's chord),
        // still looks out through it.
        let mut wide = rooms[0].clone();
        wide.ring = vec![[-5.0, -5.0], [5.2, -5.0], [5.2, 5.0], [-5.0, 5.0]];
        assert_eq!(classify_in(&[wide], [5.1, 1.6, 0.0], &[[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]]).unwrap(), vec![None, Some(0.2)]);
    }

    #[test]
    fn the_meter_opens_up_in_a_room_and_is_unchanged_outside_one() {
        let f = eo_look::LookFrame::new(13.0);
        let w = eo_sky::weather::preset(eo_sky::weather::Kind::Clear);
        let dirs: Vec<([f64; 3], f64)> = (0..15).map(|i| ([((i % 5) as f64 - 2.0) * 0.2, ((i / 5) as f64 - 1.0) * 0.2, -1.0], 1.0)).collect();
        let open = eo_sky::night::meter(&f, &w, &dirs);
        let none: Vec<([f64; 3], f64, Option<f64>)> = dirs.iter().map(|d| (d.0, d.1, None)).collect();
        assert_eq!(eo_sky::night::meter_room(&f, &w, &none), open, "no room direction: the open meter, bit for bit");
        let dim: Vec<([f64; 3], f64, Option<f64>)> = dirs.iter().map(|d| (d.0, d.1, Some(0.15))).collect();
        let indoors = eo_sky::night::meter_room(&f, &w, &dim);
        assert!(indoors > open && indoors <= eo_sky::night::METER_ROOM_MAX + 1e-9, "{indoors} vs {open}");
    }
}
