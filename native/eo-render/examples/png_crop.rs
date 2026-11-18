//! crop (and optionally scale up by an integer) a region of a capture.
//!   cargo run --release -p eo-render --example png_crop -- <in.png> <out.png> x0 y0 x1 y1 [scale]

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&a[0]).expect("open")));
    let mut r = dec.read_info().expect("png");
    let mut buf = vec![0u8; r.output_buffer_size().expect("size")];
    let info = r.next_frame(&mut buf).expect("frame");
    let (w, ch) = (info.width as usize, info.color_type.samples());
    let v: Vec<usize> = a[2..6].iter().map(|s| s.parse().unwrap()).collect();
    let k: usize = a.get(6).map(|s| s.parse().unwrap()).unwrap_or(1);
    let (cw, chh) = (v[2] - v[0], v[3] - v[1]);
    let mut out = Vec::with_capacity(cw * chh * k * k * 4);
    for y in 0..chh * k {
        for x in 0..cw * k {
            let p = ((v[1] + y / k) * w + v[0] + x / k) * ch;
            out.extend_from_slice(&[buf[p], buf[p + 1], buf[p + 2], 255]);
        }
    }
    let f = std::io::BufWriter::new(std::fs::File::create(&a[1]).unwrap());
    let mut e = png::Encoder::new(f, (cw * k) as u32, (chh * k) as u32);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&out).unwrap();
}
