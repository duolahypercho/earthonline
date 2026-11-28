//! An uninterrupted road can be exported as separate records at a bend.
//! Give both records the same cross-section there without merging their lanes,
//! traffic IDs, kerb ramps, or the sections belonging to a junction or bridge.
use crate::StreetCity;
use crate::mesh::EntryGeom;
use crate::plan::{Entry, Frame, Plan};
use std::collections::HashMap;

pub(crate) fn join(plan: &Plan, city: &StreetCity, geometry: &mut [EntryGeom]) {
    let mut ends = HashMap::<_, Vec<_>>::new();
    for (index, e) in plan.entries.iter().enumerate() {
        for (side, p) in [e.points[0], *e.points.last().unwrap()]
            .into_iter()
            .enumerate()
        {
            ends.entry(p.map(f64::to_bits))
                .or_default()
                .push((index, side));
        }
    }
    let mut normals = vec![[None; 2]; geometry.len()];
    for pair in ends.values() {
        let &[(i, a), (j, b)] = pair.as_slice() else {
            continue;
        };
        let ei = &plan.entries[i];
        let ej = &plan.entries[j];
        let way = &city.segments[ei.road].street_id;
        if i == j || way.is_none() || *way != city.segments[ej.road].street_id || ei.half != ej.half
        {
            continue;
        }
        let terminal = |e: &Entry, g: &EntryGeom, side: usize| {
            if e.structure_leg.is_some() || e.structure_ends[side].is_some() || g.stations.len() < 2
            {
                return None;
            }
            let (trim, s, kept) = if side == 0 {
                (e.trim_start, 0., g.keep[0])
            } else {
                (e.trim_end, e.length, *g.keep.last().unwrap())
            };
            (trim == 0. && kept).then(|| e.frame(s, false))
        };
        let (Some(fa), Some(fb)) = (terminal(ei, &geometry[i], a), terminal(ej, &geometry[j], b))
        else {
            continue;
        };
        let sign = if a == b { -1. } else { 1. };
        let (nx, nz) = (fa.nx + sign * fb.nx, fa.nz + sign * fb.nz);
        let den = nx * fa.nx + nz * fa.nz;
        // Same miter limit as an interior vertex of a single surveyed run.
        if den <= 2. * 0.25_f64.powi(2) || (2. - den).abs() <= 1e-12 {
            continue;
        }
        let offset = [nx / den, nz / den];
        normals[i][a] = Some(offset);
        normals[j][b] = Some(offset.map(|v| sign * v));
    }
    for ((e, g), [start, end]) in plan.entries.iter().zip(geometry).zip(normals) {
        if start.is_none() && end.is_none() {
            continue;
        }
        let rows = &mut g.stations;
        let last = rows.len() - 1;
        for (index, offset) in [(0, start), (last, end)] {
            if let Some([x, z]) = offset {
                let length = x.hypot(z);
                rows[index].nx = x / length;
                rows[index].nz = z / length;
                rows[index].miter = length;
            }
        }
        // Subdivision rows follow the two straight pavement edges. Applying
        // the miter only to the last row could fold a wide road back over a
        // nearer row, especially a ramp station a few millimetres away.
        let blend = |rows: &mut [Frame]| {
            let (a, b) = (rows[0], rows[rows.len() - 1]);
            let inner = rows.len().saturating_sub(1);
            for f in &mut rows[1..inner] {
                let cross = a.split(b, f.s);
                (f.nx, f.nz, f.miter) = (cross.nx, cross.nz, cross.miter);
            }
        };
        g.shared_ends = [start.is_some(), end.is_some()];
        let width = e.half + e.walks[0].max(e.walks[1]);
        let reach = |f: Frame| width * (f.tx * f.nx + f.tz * f.nz).abs() * f.miter;
        let first_limit = (rows[0].s + reach(rows[0])).min(e.cum[1]);
        let last_limit = (rows[last].s - reach(rows[last])).max(e.cum[e.cum.len() - 2]);
        let first_end = rows.partition_point(|f| f.s < first_limit).clamp(1, last);
        let last_start = rows
            .partition_point(|f| f.s <= last_limit)
            .saturating_sub(1)
            .min(last - 1);
        if start.is_some() && end.is_some() && first_end > last_start && e.points.len() == 2 {
            blend(rows);
        } else {
            if start.is_some() {
                blend(&mut rows[..=first_end]);
            }
            if end.is_some() {
                blend(&mut rows[last_start..]);
            }
        }
    }
}
