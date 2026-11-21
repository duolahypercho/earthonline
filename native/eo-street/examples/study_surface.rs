//! Export a local portion of the actual near street mesh for Blender lighting studies.
//! Arguments are the survey centre (east, south) and the street and horizon radii in installed metres.
//! JSON is written to standard output so an authoring tool can consume it in memory.

use anyhow::{Context, Result, ensure};
use eo_data::{City, DataPaths, ElevationGrid, groundscale};
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain, tile_key};
use serde::Serialize;
use std::io::{self, BufWriter, Write};
use std::sync::Arc;

#[derive(Serialize)]
struct Triangle {
    layer: u8,
    positions: [[f32; 3]; 3],
    colors: [u32; 3],
}

#[derive(Serialize)]
struct Neighbour<'a> {
    id: &'a str,
    perimeter: &'a [[f32; 2]],
    ground: f64,
    height: f32,
}

#[derive(Serialize)]
struct Site<'a> {
    plan_scale: f64,
    height_scale: f64,
    reference_height: f64,
    skyline: Vec<Neighbour<'a>>,
    triangles: Vec<Triangle>,
    ground_rows: Vec<Vec<[f64; 3]>>,
}

fn main() -> Result<()> {
    let args: Vec<f64> = std::env::args()
        .skip(1)
        .map(|s| s.parse())
        .collect::<Result<_, _>>()?;
    ensure!(
        args.len() == 4
            && args.iter().all(|x| x.is_finite())
            && args[2] > 0.0
            && args[3] >= args[2],
        "expected survey east, survey south and positive street and horizon radii in metres"
    );
    let [x, z] = groundscale::plan([args[0], args[1]]);
    let r = args[2];
    let horizon = args[3];
    let paths = DataPaths::discover()?;
    let city = City::load(&paths)?;
    let elevation = Arc::new(ElevationGrid::load(&paths)?);
    let options = StreetOptions::load()?;
    let rings = city
        .buildings
        .iter()
        .map(|b| b.ring.iter().map(|p| p.map(f64::from)).collect())
        .collect();
    let terrain = Terrain {
        grid: elevation.clone(),
        carpet_lift: options.open_ground_lift,
    };
    let world = StreetWorld::build(
        options,
        StreetCity::load()?,
        terrain,
        BuildingIndex::new(rings),
    );
    let mut triangles = Vec::new();
    let lo = tile_key(x - r, z - r);
    let hi = tile_key(x + r, z + r);
    // A street span's midpoint owns it. Include the adjacent tiles so a
    // sidewalk, bank or junction crossing a tile boundary reaches this window.
    for tz in lo.1 - 1..=hi.1 + 1 {
        for tx in lo.0 - 1..=hi.0 + 1 {
            let mesh = world.tile_mesh((tx, tz));
            for (indices, layer) in mesh.indices.chunks_exact(3).zip(mesh.layers) {
                let corners = [indices[0], indices[1], indices[2]].map(|i| i as usize);
                let positions = corners.map(|i| mesh.positions[i]);
                let outside = [(0, x), (2, z)].iter().any(|&(axis, center)| {
                    positions.iter().all(|p| f64::from(p[axis]) < center - r)
                        || positions.iter().all(|p| f64::from(p[axis]) > center + r)
                });
                if !outside {
                    triangles.push(Triangle {
                        layer: layer as u8,
                        positions,
                        colors: corners.map(|i| mesh.colors[i]),
                    });
                }
            }
        }
    }
    ensure!(
        !triangles.is_empty(),
        "no street surface intersects this site"
    );
    let meta = &elevation.meta;
    let cell = meta.cell_size;
    let ticks = |origin: f64, centre: f64| {
        let index = |v: f64| (v - origin) / cell - 0.5;
        let first = index(centre - r).floor() as i32;
        let last = index(centre + r).ceil() as i32;
        // Keep every grid vertex through the street study and step outward
        // on the same lattice. Shared rows meet without terrain seams.
        let outer_lo = index(centre - horizon).floor() as i32;
        let outer_hi = index(centre + horizon).ceil() as i32;
        let mut rows: Vec<i32> = (first..=last).collect();
        rows.extend((outer_lo..first).rev().skip(3).step_by(4));
        rows.extend(((last + 4)..=outer_hi).step_by(4));
        rows.sort_unstable();
        rows.into_iter()
            .map(move |i| origin + (f64::from(i) + 0.5) * cell)
    };
    let ground_rows = ticks(meta.origin_z, z)
        .map(|sz| {
            ticks(meta.origin_x, x)
                .map(|sx| {
                    let y = if (sx - x).abs().max((sz - z).abs()) <= r + cell {
                        world.carpet_y(sx, sz)
                    } else {
                        elevation.ground(sx, sz)
                    };
                    [sx, y, sz]
                })
                .collect()
        })
        .collect();
    let mut skyline = Vec::new();
    for building in &city.buildings {
        let Some(first) = building.ring.first() else {
            continue;
        };
        if building.ring.len() < 3
            || (f64::from(first[0]) - x).hypot(f64::from(first[1]) - z) > horizon
        {
            continue;
        }
        let y = building
            .ring
            .iter()
            .map(|p| elevation.ground(f64::from(p[0]), f64::from(p[1])))
            .fold(f64::NEG_INFINITY, f64::max);
        skyline.push(Neighbour {
            id: &building.id,
            perimeter: &building.ring,
            ground: y,
            height: building.height,
        });
    }
    let site = Site {
        skyline,
        plan_scale: groundscale::scale(),
        height_scale: groundscale::height_scale(),
        reference_height: elevation.ground(x, z),
        triangles,
        ground_rows,
    };
    let mut writer = BufWriter::new(io::stdout().lock());
    serde_json::to_writer(&mut writer, &site).context("write street geometry")?;
    writer.flush().context("finish writing street geometry")
}
