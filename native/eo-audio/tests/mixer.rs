//! The mixer, rendered offline (no device, nothing sounds): levels, panning,
//! distance, Doppler, the voice limit and its priorities, virtual voices,
//! buses, the limiter, streaming against the in-memory path, and loops.

use eo_audio::bank::{Bank, Clip};
use eo_audio::mixer::{MAX_VOICES, Source};
use eo_audio::{Bus, Engine, Falloff, Listener, Play, Spatial};
use std::sync::Arc;

const RATE: u32 = 48_000;

fn sine(freq: f32, secs: f32, amp: f32, rate: u32) -> Vec<i16> {
    (0..(secs * rate as f32) as usize).map(|i| ((i as f32 / rate as f32 * freq * std::f32::consts::TAU).sin() * amp * 32767.0) as i16).collect()
}

fn clip(role: &str, data: Vec<i16>, rate: u32) -> Clip {
    Clip { id: role.into(), role: role.into(), rate, frames: data.len() as u32, gain: 1.0, data: Some(data.into()), path: Default::default(), channels: 1, slices: vec![], loop_region: None, lufs: -20.0 }
}

fn render(off: &mut eo_audio::Offline, secs: f32) -> Vec<f32> {
    let mut v = vec![];
    off.render(&mut v, (secs * RATE as f32) as usize);
    v
}

fn rms(v: &[f32], ch: usize) -> f32 {
    let s: f32 = v.iter().skip(ch).step_by(2).map(|x| x * x).sum();
    (s / (v.len() / 2) as f32).sqrt()
}

fn two_d(clip: u32, gain: f32, pan: f32) -> Play {
    Play { source: Source::Clip { clip, start: 0, len: 0, looping: true }, bus: Bus::Effects, priority: 7, gain, pitch: 1.0, pan, spatial: None, fade_in_s: 0.0, lowpass_hz: 20_000.0 }
}

fn at(clip: u32, pos: [f32; 3], vel: [f32; 3], falloff: Falloff, priority: u8) -> Play {
    Play { spatial: Some(Spatial { pos, vel, falloff, doppler: 1.0 }), priority, ..two_d(clip, 1.0, 0.0) }
}

#[test]
fn centre_pan_is_minus_three_db_per_side() {
    let bank = Arc::new(Bank::from_clips(vec![clip("tone", sine(1000.0, 2.0, 0.5, RATE), RATE)]));
    let (mut e, mut off) = Engine::offline(bank, RATE);
    e.play(two_d(0, 1.0, 0.0));
    let v = render(&mut off, 1.0);
    let tail = &v[v.len() / 2..];
    let want = 0.5 / 2f32.sqrt() / 2f32.sqrt(); // sine RMS x -3 dB
    assert!((rms(tail, 0) - want).abs() < 0.01 * want * 3.0, "L {} want {want}", rms(tail, 0));
    assert!((rms(tail, 0) - rms(tail, 1)).abs() < 1e-3);
}

#[test]
fn distance_and_side() {
    let bank = Arc::new(Bank::from_clips(vec![clip("tone", sine(500.0, 3.0, 0.5, RATE), RATE)]));
    let level = |pos: [f32; 3]| {
        let (mut e, mut off) = Engine::offline(bank.clone(), RATE);
        e.send(eo_audio::Cmd::Listener(Listener::default()));
        e.play(at(0, pos, [0.0; 3], Falloff::new(2.0, 1000.0, 1.0), 7));
        let v = render(&mut off, 1.0);
        let t = &v[v.len() / 2..];
        (rms(t, 0), rms(t, 1))
    };
    // Ahead (-z with the default listener) at 4 m and 8 m: -6 dB per doubling.
    let (l4, r4) = level([0.0, 0.0, -4.0]);
    let (l8, _) = level([0.0, 0.0, -8.0]);
    let db = 20.0 * (l8 / l4).log10();
    assert!((db + 6.02).abs() < 0.3, "{db} dB");
    assert!((l4 - r4).abs() < 1e-3 * l4.max(1e-6) * 10.0, "ahead is centred");
    // On the right (+x): the right channel carries it.
    let (l, r) = level([6.0, 0.0, 0.0]);
    assert!(r > 20.0 * l, "right {r} left {l}");
}

/// Zero crossings per second of the left channel.
fn freq(v: &[f32]) -> f32 {
    let l: Vec<f32> = v.iter().step_by(2).copied().collect();
    let n = l.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    n as f32 / (l.len() as f32 / RATE as f32)
}

#[test]
fn doppler_shifts_a_passing_car() {
    let bank = Arc::new(Bank::from_clips(vec![clip("tone", sine(400.0, 4.0, 0.5, RATE), RATE)]));
    let run = |vel: [f32; 3]| {
        let (mut e, mut off) = Engine::offline(bank.clone(), RATE);
        e.send(eo_audio::Cmd::Listener(Listener::default()));
        // 60 m ahead, held there, with a velocity toward (or away from) the listener.
        e.play(at(0, [0.0, 0.0, -60.0], vel, Falloff::new(2.0, 2000.0, 1.0), 7));
        let v = render(&mut off, 1.5);
        freq(&v[v.len() / 3..])
    };
    let still = run([0.0; 3]);
    let toward = run([0.0, 0.0, 20.0]);
    let away = run([0.0, 0.0, -20.0]);
    assert!((still - 400.0).abs() < 3.0, "{still}");
    let want_up = 400.0 * 343.0 / (343.0 - 20.0);
    let want_down = 400.0 * 343.0 / (343.0 + 20.0);
    assert!((toward - want_up).abs() < 4.0, "{toward} vs {want_up}");
    assert!((away - want_down).abs() < 4.0, "{away} vs {want_down}");
}

#[test]
fn voice_limit_keeps_the_important_and_the_near() {
    let bank = Arc::new(Bank::from_clips(vec![clip("tone", sine(300.0, 2.0, 0.2, RATE), RATE)]));
    let (mut e, mut off) = Engine::offline(bank, RATE);
    e.send(eo_audio::Cmd::Listener(Listener::default()));
    // 150 louder sources at priority 1, all hard left, then one quiet priority-7 source hard right.
    for i in 0..150 {
        e.play(at(0, [-30.0 - i as f32 * 0.1, 0.0, 0.0], [0.0; 3], Falloff::new(1.0, 500.0, 1.0), 1));
    }
    let vip = e.play(at(0, [200.0, 0.0, 0.0], [0.0; 3], Falloff::new(1.0, 1000.0, 1.0), 7));
    assert!(vip.is_some());
    let _ = render(&mut off, 0.5);
    let st = &e.stats;
    use std::sync::atomic::Ordering::Relaxed;
    assert_eq!(st.active.load(Relaxed), 151);
    assert!(st.mixed.load(Relaxed) as usize <= MAX_VOICES, "mixed {}", st.mixed.load(Relaxed));
    assert!(st.virtual_.load(Relaxed) >= 151 - MAX_VOICES as u32);
    // The far priority-7 source is heard: the right channel is it alone (0.2 / 200 m, sine RMS).
    let v = render(&mut off, 0.5);
    let r = rms(&v[v.len() / 2..], 1);
    let want = 0.2 / 200.0 / 2f32.sqrt();
    assert!((r - want).abs() < 0.15 * want, "the important source is mixed ({r}, want {want})");
}

#[test]
fn virtual_one_shots_end_on_time() {
    let bank = Arc::new(Bank::from_clips(vec![clip("blip", sine(300.0, 0.5, 0.2, RATE), RATE)]));
    let (mut e, mut off) = Engine::offline(bank, RATE);
    for _ in 0..(MAX_VOICES + 40) {
        e.play(Play { source: Source::Clip { clip: 0, start: 0, len: 0, looping: false }, ..two_d(0, 0.01, 0.0) });
    }
    let _ = render(&mut off, 0.2);
    use std::sync::atomic::Ordering::Relaxed;
    assert_eq!(e.stats.active.load(Relaxed) as usize, MAX_VOICES + 40);
    let _ = render(&mut off, 0.5);
    assert_eq!(e.stats.active.load(Relaxed), 0, "every one-shot ended, mixed or virtual");
}

#[test]
fn bus_gain_and_master_limiter() {
    let bank = Arc::new(Bank::from_clips(vec![clip("tone", sine(200.0, 2.0, 0.9, RATE), RATE)]));
    let (mut e, mut off) = Engine::offline(bank, RATE);
    for _ in 0..20 {
        e.play(two_d(0, 1.0, 0.0));
    }
    let v = render(&mut off, 0.5);
    let peak = v.iter().fold(0f32, |m, x| m.max(x.abs()));
    assert!(peak <= 0.9001, "limited to -0.9 dBFS: {peak}");
    assert!(e.stats.limited_frames.load(std::sync::atomic::Ordering::Relaxed) > 0);
    e.send(eo_audio::Cmd::Bus { bus: Bus::Effects, gain: 0.0 });
    let _ = render(&mut off, 0.3);
    let v = render(&mut off, 0.2);
    assert!(v.iter().all(|x| x.abs() < 1e-4), "a silent bus is silent");
}

#[test]
fn streamed_clip_plays_like_the_decoded_one() {
    let dir = std::env::temp_dir().join(format!("eo-audio-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bed.wav");
    let data = sine(220.0, 3.0, 0.4, 44_100);
    let mut w = hound::WavWriter::create(&path, hound::WavSpec { channels: 1, sample_rate: 44_100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int }).unwrap();
    for s in &data {
        w.write_sample(*s).unwrap();
    }
    w.finalize().unwrap();
    let mem = clip("mem", data.clone(), 44_100);
    let streamed = Clip { data: None, path: path.clone(), role: "stream".into(), ..clip("stream", vec![0; data.len()], 44_100) };
    let bank = Arc::new(Bank::from_clips(vec![mem, streamed]));
    let play = |k: u32| {
        let (mut e, mut off) = Engine::offline(bank.clone(), RATE);
        e.play(Play { source: Source::Clip { clip: k, start: 0, len: 0, looping: false }, ..two_d(k, 1.0, -0.3) });
        render(&mut off, 2.0)
    };
    let a = play(0);
    let b = play(1);
    // The stream may start a few ms late (its first chunk); compare after aligning on the first loud sample.
    let first = |v: &[f32]| v.iter().position(|x| x.abs() > 0.01).unwrap();
    let (fa, fb) = (first(&a), first(&b));
    assert!(fb.abs_diff(fa) <= 2 * 512, "stream start lag {} frames", (fb as i64 - fa as i64) / 2);
    let n = 2 * 48_000;
    let err: f32 = a[fa..fa + n].iter().zip(&b[fb..fb + n]).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
    assert!(err < 2e-3, "streamed and decoded differ by {err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_loop_runs_past_its_end_without_a_click() {
    // A 0.5 s tone looped for 2 s: no jump between neighbouring samples bigger than the tone's own slope allows.
    let bank = Arc::new(Bank::from_clips(vec![clip("loop", sine(250.0, 0.5, 0.5, RATE), RATE)]));
    let (mut e, mut off) = Engine::offline(bank, RATE);
    e.play(two_d(0, 1.0, 0.0));
    let v = render(&mut off, 2.0);
    let l: Vec<f32> = v.iter().step_by(2).copied().collect();
    let max_step = l.windows(2).skip(4800).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
    let slope = 0.5 * 0.7072 * std::f32::consts::TAU * 250.0 / RATE as f32;
    assert!(max_step < 2.5 * slope, "step {max_step} vs slope {slope}");
    assert!(rms(&v[v.len() * 3 / 4..], 0) > 0.1, "still playing after 4 loops");
}

#[test]
fn render_thread_commands_are_cheap() {
    let bank = Arc::new(Bank::from_clips(vec![clip("tone", sine(300.0, 1.0, 0.2, RATE), RATE)]));
    let (mut e, mut off) = Engine::offline(bank, RATE);
    let hs: Vec<_> = (0..64).map(|i| e.play(at(0, [i as f32, 0.0, 0.0], [0.0; 3], Falloff::new(2.0, 200.0, 1.0), 3))).collect();
    let _ = render(&mut off, 0.05);
    let t = std::time::Instant::now();
    for _ in 0..100 {
        for h in &hs {
            e.update(*h, [1.0, 0.0, 2.0], [3.0, 0.0, 0.0], 0.5, 1.0, 9_000.0);
        }
        off.mixer.render(&mut [0f32; 0], 2);
    }
    let per_frame_us = t.elapsed().as_secs_f64() * 1e6 / 100.0;
    eprintln!("64 updates a frame: {per_frame_us:.1} us (render thread side plus draining)");
    assert!(per_frame_us < 500.0);
}
