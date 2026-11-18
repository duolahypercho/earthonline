//! Lists the steepest street ways in the data (20 m window, grade measured
//! within each way) and the route the walk test and bench chain from them.
//!   cargo run --release -p eo-player --example steep_streets

fn main() -> anyhow::Result<()> {
    let paths = eo_data::DataPaths::discover()?;
    let city = eo_data::City::load(&paths)?;
    let elevation = eo_data::ElevationGrid::load(&paths)?;
    let ways = eo_player::routes::steep_ways(&city, &elevation, 20.0);
    for (i, g, at) in ways.iter().take(40) {
        let r = &city.roads[*i];
        let (p, inter) = eo_player::routes::chain_route(&city, *i, 2200.0);
        println!(
            "{:28} {:12} {:5.1} %  at ({:.0}, {:.0}); chained {:.0} m, {} intersections",
            r.name,
            r.highway,
            g * 100.0,
            at[0],
            at[1],
            eo_player::routes::polyline_length(&p),
            inter.len()
        );
    }
    let route = eo_player::routes::walk_route(&city, &elevation);
    println!(
        "walk route (from {}): {:.0} m, {} intersections, steepest {:.1} %, from ({:.0}, {:.0}) to ({:.0}, {:.0})",
        city.roads[route.start_way].name,
        eo_player::routes::polyline_length(&route.path),
        route.intersections.len(),
        route.steepest_grade * 100.0,
        route.path[0][0],
        route.path[0][1],
        route.path.last().unwrap()[0],
        route.path.last().unwrap()[1]
    );
    Ok(())
}
