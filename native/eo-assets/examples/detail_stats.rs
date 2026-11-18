//! per-cell channel statistics of facade-detail-v1 (normal x, y and the
//! cavity channel: mean, sd, percentiles), the numbers the detail's albedo,
//! roughness and occlusion response follow from.
//!   EO_REPO_ROOT=... cargo run --release -p eo-assets --example detail_stats
fn main() {
    let root = std::path::PathBuf::from(std::env::var("EO_REPO_ROOT").unwrap());
    let d = eo_assets::facade_detail::load_cached(&root, None).unwrap();
    let (w, h) = (d.width as usize, d.height as usize);
    let l0 = &d.levels[0];
    for cell in 0..6 {
        let col = cell % 3;
        let row = cell / 3; // row 0 at image bottom
        let x0 = col * 512 + 16;
        let y0 = h - (row + 1) * 512 + 16;
        for c in 0..3 {
            let mut v: Vec<f64> = vec![];
            for y in y0..y0 + 480 {
                for x in x0..x0 + 480 {
                    v.push(l0[(y * w + x) * 4 + c] as f64 / 255.0);
                }
            }
            let n = v.len() as f64;
            let m = v.iter().sum::<f64>() / n;
            let sd = (v.iter().map(|a| (a - m) * (a - m)).sum::<f64>() / n).sqrt();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let p = |q: f64| v[((n - 1.0) * q) as usize];
            println!("cell {cell} ch {c}: mean {m:.4} sd {sd:.4} p1 {:.3} p5 {:.3} p50 {:.3} p95 {:.3} p99 {:.3}", p(0.01), p(0.05), p(0.5), p(0.95), p(0.99));
        }
    }
}
