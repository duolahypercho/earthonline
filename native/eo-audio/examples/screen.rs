//! The automatic screen (nobody listens during it): for every clip, the format
//! (rate, channels, bits, length), the peak and clipping, integrated loudness
//! (EBU R 128 / ITU-R BS.1770, the `ebur128` crate), the noise floor and the
//! loud level (10th and 95th percentile of 50 ms RMS), DC offset, the silent
//! share, how steady it is (spread of 100 ms RMS in dB, for loops and beds),
//! the onsets of one-shot events (steps), and a steady loop window.
//!
//!   cargo run --release -p eo-audio --example screen -- <root> <list.json> <out.json>
//!
//! `list.json`: `[{"path": "fsd50k/clips/dev/123.wav", "roles": ["footsteps_concrete"]}, ...]`
//! (paths under `root`). Reads only; runs on 8 threads.

use serde_json::{Value, json};
use std::path::Path;

fn db(x: f64) -> f64 {
    if x <= 1e-12 { -240.0 } else { 20.0 * x.log10() }
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((p / 100.0) * (v.len() - 1) as f64).round() as usize]
}

/// Frames of `x` (mono) RMS over windows of `w` samples.
fn rms_frames(x: &[f32], w: usize) -> Vec<f64> {
    x.chunks(w).filter(|c| c.len() == w).map(|c| (c.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>() / w as f64).sqrt()).collect()
}

/// Step onsets: 5 ms envelope rising 9 dB over its minimum of the last 120 ms and
/// 14 dB over the floor, at least 180 ms apart. Returns [start, len] slices.
fn onsets(x: &[f32], rate: u32, floor_db: f64) -> Vec<[u32; 2]> {
    let w = (rate as usize / 200).max(1);
    let env: Vec<f64> = rms_frames(x, w).into_iter().map(db).collect();
    let look = 24; // 120 ms of 5 ms frames
    let refr = 36; // 180 ms
    let mut hits = vec![];
    let mut last: i64 = -1000;
    for i in 1..env.len() {
        let lo = env[i.saturating_sub(look)..i].iter().cloned().fold(f64::MAX, f64::min);
        if env[i] - lo >= 9.0 && env[i] - floor_db >= 14.0 && (i as i64 - last) >= refr {
            hits.push(i);
            last = i as i64;
        }
    }
    let max_len = (0.35 * rate as f64) as usize;
    let pre = (0.006 * rate as f64) as usize;
    let mut out = vec![];
    for (k, &h) in hits.iter().enumerate() {
        let start = (h * w).saturating_sub(pre);
        let next = hits.get(k + 1).map(|&n| (n * w).saturating_sub(pre + w)).unwrap_or(x.len());
        let len = (next.min(start + max_len).min(x.len())).saturating_sub(start);
        if len >= (0.08 * rate as f64) as usize {
            out.push([start as u32, len as u32]);
        }
    }
    // Keep the steps within 12 dB of the loudest one.
    let peaks: Vec<f64> = out.iter().map(|s| db(x[s[0] as usize..(s[0] + s[1]) as usize].iter().fold(0f32, |m, v| m.max(v.abs())) as f64)).collect();
    let top = peaks.iter().cloned().fold(f64::MIN, f64::max);
    out.into_iter().zip(peaks).filter(|(_, p)| *p >= top - 12.0).map(|(s, _)| s).collect()
}

/// The steadiest window of `secs` in the first `within` seconds: [start, len] and its RMS spread (dB).
fn steady_window(x: &[f32], rate: u32, secs: f64, within: f64) -> Option<([u32; 2], f64)> {
    let w = (rate as f64 * 0.1) as usize;
    let fr: Vec<f64> = rms_frames(x, w).into_iter().map(db).collect();
    let n = (secs / 0.1) as usize;
    let lim = ((within / 0.1) as usize).min(fr.len());
    if lim < n + 3 {
        return None;
    }
    let mut best: Option<(usize, f64)> = None;
    for s in 2..=lim - n {
        let win = &fr[s..s + n];
        let m = win.iter().sum::<f64>() / n as f64;
        if m < -55.0 {
            continue;
        }
        let sd = (win.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / n as f64).sqrt();
        if best.is_none_or(|b| sd < b.1) {
            best = Some((s, sd));
        }
    }
    best.map(|(s, sd)| ([(s * w) as u32, (n * w) as u32], sd))
}

fn screen(root: &Path, rel: &str, roles: &[String]) -> Value {
    let p = rel.split('/').fold(root.to_path_buf(), |a, c| a.join(c));
    let r = match hound::WavReader::open(&p) {
        Ok(r) => r,
        Err(e) => return json!({ "path": rel, "error": format!("{e}") }),
    };
    let spec = r.spec();
    let frames = r.duration();
    let bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    let rate = spec.sample_rate;
    let ch = spec.channels as usize;
    let inter: Vec<f32>;
    let full = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, _) => 1.0f32,
        (_, b) => ((1u64 << (b - 1)) - 1) as f32 / (1u64 << (b - 1)) as f32,
    };
    let rd = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, _) => r.into_samples::<f32>().map(|s| s.map_err(|e| e.to_string())).collect::<Result<Vec<_>, _>>(),
        (_, 16) => r.into_samples::<i16>().map(|s| s.map(|v| v as f32 / 32768.0).map_err(|e| e.to_string())).collect(),
        (_, b) => {
            let sc = 1.0 / (1u64 << (b - 1)) as f32;
            r.into_samples::<i32>().map(|s| s.map(|v| v as f32 * sc).map_err(|e| e.to_string())).collect()
        }
    };
    match rd {
        Ok(v) => inter = v,
        Err(e) => return json!({ "path": rel, "error": e }),
    }
    let mono: Vec<f32> = inter.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect();
    let peak = inter.iter().fold(0f32, |m, v| m.max(v.abs())) as f64;
    let clip_level = full * 0.999;
    let clipped = inter.iter().filter(|v| v.abs() >= clip_level).count();
    // Runs of 3+ consecutive full-scale samples on one channel.
    let mut runs = 0usize;
    for c in 0..ch {
        let mut k = 0;
        for i in (c..inter.len()).step_by(ch) {
            if inter[i].abs() >= clip_level {
                k += 1;
                if k == 3 {
                    runs += 1;
                }
            } else {
                k = 0;
            }
        }
    }
    let dc = mono.iter().map(|&v| v as f64).sum::<f64>() / mono.len().max(1) as f64;
    let mut r50: Vec<f64> = rms_frames(&mono, (rate as usize / 20).max(1)).into_iter().map(db).collect();
    let silent = r50.iter().filter(|&&v| v < -60.0).count() as f64 / r50.len().max(1) as f64;
    let floor = pct(&mut r50.clone(), 10.0);
    let loud = pct(&mut r50, 95.0);
    let mut r100: Vec<f64> = rms_frames(&mono, (rate as usize / 10).max(1)).into_iter().map(db).filter(|v| *v > -70.0).collect();
    let spread = if r100.len() > 3 {
        let m = r100.iter().sum::<f64>() / r100.len() as f64;
        (r100.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / r100.len() as f64).sqrt()
    } else {
        f64::NAN
    };
    let lufs = {
        let mut e = ebur128::EbuR128::new(ch as u32, rate, ebur128::Mode::I).unwrap();
        let _ = e.add_frames_f32(&inter);
        e.loudness_global().unwrap_or(f64::NEG_INFINITY)
    };
    let steppy = roles.iter().any(|r| r.starts_with("footsteps"));
    let slices = if steppy { onsets(&mono, rate, floor) } else { vec![] };
    let loopy = roles.iter().any(|r| r.starts_with("engine") || r == "tyre_squeal" || r == "sf_cable_car");
    let window = if loopy { steady_window(&mono, rate, 3.0, 10.0) } else { None };
    let _ = pct(&mut r100, 50.0);
    json!({
        "path": rel, "bytes": bytes, "rate": rate, "channels": ch, "bits": spec.bits_per_sample,
        "float": spec.sample_format == hound::SampleFormat::Float, "frames": frames,
        "duration_s": frames as f64 / rate as f64, "peak_db": db(peak), "clipped_samples": clipped,
        "clipped_frac": clipped as f64 / inter.len().max(1) as f64, "clip_runs": runs, "lufs": lufs,
        "floor_db": floor, "loud_db": loud, "dc": dc, "silent_frac": silent, "spread_db": spread,
        "slices": slices, "steady": window.map(|(w, sd)| json!({ "start": w[0], "len": w[1], "spread_db": sd })),
    })
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() != 3 {
        eprintln!("usage: screen <root> <list.json> <out.json>");
        std::process::exit(2);
    }
    let root = Path::new(&a[0]);
    let list: Vec<Value> = serde_json::from_slice(&std::fs::read(&a[1]).expect("list")).expect("list json");
    let items: Vec<(String, Vec<String>)> = list.iter().map(|v| (v["path"].as_str().unwrap().to_string(), v["roles"].as_array().map(|r| r.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default())).collect();
    let t = std::time::Instant::now();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let out = std::sync::Mutex::new(vec![Value::Null; items.len()]);
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= items.len() {
                    break;
                }
                let v = screen(root, &items[i].0, &items[i].1);
                out.lock().unwrap()[i] = v;
            });
        }
    });
    let out = out.into_inner().unwrap();
    std::fs::write(&a[2], serde_json::to_vec(&out).unwrap()).expect("write");
    eprintln!("screened {} clips in {:.1} s", out.len(), t.elapsed().as_secs_f64());
}
