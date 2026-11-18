//! box statistics of a capture (sRGB means, linear luma, chroma B/R),
//! for matched before/after captures.
//!
//!   cargo run --release -p eo-render --example png_box -- <png> x0 y0 x1 y1 [x0 y0 x1 y1 ...]
//!   cargo run --release -p eo-render --example png_box -- <png> --grid 8 6   (block means)

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = &args[0];
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).expect("open")));
    let mut r = dec.read_info().expect("png");
    let mut buf = vec![0u8; r.output_buffer_size().expect("size")];
    let info = r.next_frame(&mut buf).expect("frame");
    let (w, h) = (info.width as usize, info.height as usize);
    let ch = info.color_type.samples();
    let lin = |c: f64| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    let stats = |x0: usize, y0: usize, x1: usize, y1: usize| {
        let (mut s, mut n, mut luma) = ([0f64; 3], 0f64, 0f64);
        for y in y0..y1.min(h) {
            for x in x0..x1.min(w) {
                let p = (y * w + x) * ch;
                let c = [buf[p] as f64, buf[p + 1] as f64, buf[p + 2] as f64];
                for k in 0..3 {
                    s[k] += c[k];
                }
                luma += 0.2126 * lin(c[0] / 255.0) + 0.7152 * lin(c[1] / 255.0) + 0.0722 * lin(c[2] / 255.0);
                n += 1.0;
            }
        }
        let m = s.map(|v| v / n.max(1.0));
        (m, luma / n.max(1.0))
    };
    if args.get(1).map(|s| s.as_str()) == Some("--grid") {
        let (gx, gy): (usize, usize) = (args[2].parse().unwrap(), args[3].parse().unwrap());
        for j in 0..gy {
            let row: Vec<String> = (0..gx)
                .map(|i| {
                    let (m, _) = stats(i * w / gx, j * h / gy, (i + 1) * w / gx, (j + 1) * h / gy);
                    format!("{:3.0} {:3.0} {:3.0}", m[0], m[1], m[2])
                })
                .collect();
            println!("{}", row.join(" | "));
        }
        return;
    }
    for b in args[1..].chunks(4) {
        let v: Vec<usize> = b.iter().map(|s| s.parse().unwrap()).collect();
        let (m, luma) = stats(v[0], v[1], v[2], v[3]);
        println!("box {:?}: sRGB ({:.1}, {:.1}, {:.1}) linear luma {:.4} B/R {:.2}", v, m[0], m[1], m[2], luma, m[2] / m[0].max(1e-6));
    }
}
