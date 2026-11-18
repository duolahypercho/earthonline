//! per-pixel linear-luma ratio a / b of two matched captures over a box
//! (percentiles), to isolate one shading term (b = the term switched off).
//!   cargo run --release -p eo-render --example png_ratio -- <a.png> <b.png> x0 y0 x1 y1

fn load(p: &str) -> (usize, usize, Vec<u8>) {
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(p).expect("open")));
    let mut r = dec.read_info().expect("png");
    let mut buf = vec![0u8; r.output_buffer_size().expect("size")];
    let info = r.next_frame(&mut buf).expect("frame");
    (info.width as usize, info.color_type.samples(), buf)
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (w, ca, ba) = load(&a[0]);
    let (_, cb, bb) = load(&a[1]);
    let v: Vec<usize> = a[2..6].iter().map(|s| s.parse().unwrap()).collect();
    let lin = |c: u8| {
        let c = c as f64 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let luma = |b: &[u8], p: usize| 0.2126 * lin(b[p]) + 0.7152 * lin(b[p + 1]) + 0.0722 * lin(b[p + 2]);
    let mut r = vec![];
    let (mut la, mut lb) = (vec![], vec![]);
    for y in v[1]..v[3] {
        for x in v[0]..v[2] {
            let (pa, pb) = ((y * w + x) * ca, (y * w + x) * cb);
            let (x1, x2) = (luma(&ba, pa), luma(&bb, pb));
            la.push(x1);
            lb.push(x2);
            r.push(x1 / x2.max(1e-6));
        }
    }
    let pct = |v: &mut Vec<f64>, q: f64| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[((v.len() - 1) as f64 * q) as usize]
    };
    let sd = |v: &Vec<f64>| {
        let m = v.iter().sum::<f64>() / v.len() as f64;
        ((v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / v.len() as f64).sqrt() / m, m)
    };
    let (ra, ma) = sd(&la);
    let (rb, mb) = sd(&lb);
    println!("a luma mean {ma:.4} cv {ra:.3}; b luma mean {mb:.4} cv {rb:.3}");
    println!(
        "ratio a/b: p1 {:.3} p5 {:.3} p25 {:.3} p50 {:.3} p75 {:.3} p95 {:.3} p99 {:.3}",
        pct(&mut r, 0.01),
        pct(&mut r, 0.05),
        pct(&mut r, 0.25),
        pct(&mut r, 0.5),
        pct(&mut r, 0.75),
        pct(&mut r, 0.95),
        pct(&mut r, 0.99)
    );
}
