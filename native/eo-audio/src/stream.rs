//! Streaming: the long clips (beds of ambience, rain and wind) are read from
//! disk ahead of the mixer by one worker thread into wait-free rings of
//! fixed-size chunks, one ring per stream slot. The mixer asks for a clip
//! range through the slot's request ring and reads chunks tagged with the
//! request's generation (a chunk of an earlier request is dropped).

use crate::bank::{Bank, ClipId};
use std::fs::File;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

/// Samples per chunk (mono).
pub const CHUNK: usize = 512;
/// Chunks read ahead per slot: 48 x 512 = 24,576 frames, 0.56 s at 44.1 kHz.
pub const RING_CHUNKS: usize = 48;
/// Streams playing at once (beds use two each for a stereo image, plus crossfades).
pub const MAX_STREAMS: usize = 20;
/// A request that frees the slot.
pub const STOP_CLIP: ClipId = ClipId::MAX;

#[derive(Clone, Copy)]
pub struct Chunk {
    pub generation: u32,
    pub len: u16,
    pub last: bool,
    pub data: [f32; CHUNK],
}

#[derive(Clone, Copy, Debug)]
pub struct Request {
    pub generation: u32,
    pub clip: ClipId,
    pub start: u32,
    pub end: u32,
}

/// The mixer's ends of one slot.
pub struct SlotEnds {
    pub req: rtrb::Producer<Request>,
    pub chunks: rtrb::Consumer<Chunk>,
}

struct Reader {
    generation: u32,
    wav: hound::WavReader<BufReader<File>>,
    channels: usize,
    bits: u16,
    float: bool,
    pos: u32,
    end: u32,
}

struct WorkerSlot {
    req: rtrb::Consumer<Request>,
    chunks: rtrb::Producer<Chunk>,
    reader: Option<Reader>,
}

/// The worker's side: owns the open files.
pub struct StreamWorker {
    slots: Vec<WorkerSlot>,
    bank: Arc<OnceLock<Arc<Bank>>>,
    pub errors: u32,
}

pub fn slots(bank: Arc<OnceLock<Arc<Bank>>>) -> (Vec<SlotEnds>, StreamWorker) {
    let mut ends = Vec::with_capacity(MAX_STREAMS);
    let mut ws = Vec::with_capacity(MAX_STREAMS);
    for _ in 0..MAX_STREAMS {
        let (rp, rc) = rtrb::RingBuffer::new(8);
        let (cp, cc) = rtrb::RingBuffer::new(RING_CHUNKS);
        ends.push(SlotEnds { req: rp, chunks: cc });
        ws.push(WorkerSlot { req: rc, chunks: cp, reader: None });
    }
    (ends, StreamWorker { slots: ws, bank, errors: 0 })
}

/// Bytes the stream rings hold (for the memory report).
pub fn ring_bytes() -> usize {
    MAX_STREAMS * (RING_CHUNKS * std::mem::size_of::<Chunk>() + 8 * std::mem::size_of::<Request>())
}

impl StreamWorker {
    /// Take new requests and fill every ring as far as it goes. Returns chunks written.
    pub fn pump(&mut self) -> usize {
        let Some(bank) = self.bank.get().cloned() else { return 0 };
        let mut written = 0;
        for s in &mut self.slots {
            let mut newest = None;
            while let Ok(r) = s.req.pop() {
                newest = Some(r);
            }
            if let Some(r) = newest {
                s.reader = None;
                if r.clip != STOP_CLIP {
                    match open(&bank, r) {
                        Some(rd) => s.reader = Some(rd),
                        None => {
                            self.errors += 1;
                            // An empty last chunk tells the mixer the voice has ended.
                            let _ = s.chunks.push(Chunk { generation: r.generation, len: 0, last: true, data: [0.0; CHUNK] });
                        }
                    }
                }
            }
            while let Some(rd) = s.reader.as_mut() {
                if s.chunks.slots() == 0 {
                    break;
                }
                let mut c = Chunk { generation: rd.generation, len: 0, last: false, data: [0.0; CHUNK] };
                let want = ((rd.end - rd.pos) as usize).min(CHUNK);
                let n = read_mono(rd, &mut c.data[..want]);
                rd.pos += n as u32;
                c.len = n as u16;
                c.last = n < want || rd.pos >= rd.end;
                let done = c.last;
                let _ = s.chunks.push(c);
                written += 1;
                if done {
                    s.reader = None;
                }
            }
        }
        written
    }

    /// The worker thread: pump every 4 ms until `stop`.
    pub fn spawn(mut self, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
        std::thread::Builder::new()
            .name("eo-audio-stream".into())
            .spawn(move || {
                crate::worker_started();
                while !stop.load(Ordering::Relaxed) {
                    self.pump();
                    std::thread::sleep(std::time::Duration::from_millis(4));
                }
            })
            .expect("spawn audio stream thread")
    }
}

fn open(bank: &Bank, r: Request) -> Option<Reader> {
    let clip = bank.clip(r.clip)?;
    let f = File::open(&clip.path).ok()?;
    let mut wav = hound::WavReader::new(BufReader::with_capacity(64 * 1024, f)).ok()?;
    let spec = wav.spec();
    if spec.sample_rate != clip.rate || wav.duration() != clip.frames {
        return None; // the file changed since it was verified
    }
    let end = r.end.min(clip.frames);
    let start = r.start.min(end);
    wav.seek(start).ok()?;
    Some(Reader { generation: r.generation, wav, channels: spec.channels.max(1) as usize, bits: spec.bits_per_sample, float: spec.sample_format == hound::SampleFormat::Float, pos: start, end })
}

fn read_mono(rd: &mut Reader, out: &mut [f32]) -> usize {
    let ch = rd.channels;
    let inv = 1.0 / ch as f32;
    let mut n = 0;
    macro_rules! pull {
        ($t:ty, $scale:expr) => {{
            let mut it = rd.wav.samples::<$t>();
            'frames: for o in out.iter_mut() {
                let mut acc = 0.0f32;
                for _ in 0..ch {
                    match it.next() {
                        Some(Ok(v)) => acc += v as f32 * $scale,
                        _ => break 'frames,
                    }
                }
                *o = acc * inv;
                n += 1;
            }
        }};
    }
    if rd.float {
        pull!(f32, 1.0);
    } else if rd.bits == 16 {
        pull!(i16, 1.0 / 32768.0);
    } else {
        let s = 1.0 / (1u64 << (rd.bits - 1)) as f32;
        pull!(i32, s);
    }
    n
}
