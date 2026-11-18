//! the CC0 scanned sets at run time. Whether they can be used (the
//! downloads present with the manifest's sizes, and BC compression on the
//! adapter), the cook threads (eo-assets `cc0::load_cached`: the cache under
//! `native/cache/cc0/`, or verify + decode + BC7/BC5 once), their uploads
//! through the budgeted queue (two sets queued at a time, so at most a few of
//! them are held in memory), and the tables once every layer is on the GPU:
//! only then does the renderer switch the sets on (`RUN_ON`), so a frame never
//! samples a half-written array.

use eo_assets::cc0 as ac;
use eo_render::authored::MaterialTable;
use eo_render::cc0 as rc;
use eo_render::upload::{Job, UploadQueue};
use eo_render::{GpuContext, Renderer, wgpu};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Instant;

/// Upload job tag (below the other streams' bits 58-62).
pub const TAG_CC0: u64 = 1 << 57;

/// Can the sets be used: the adapter samples BC, and every source file is present.
pub fn availability(ctx: &GpuContext) -> Result<(), String> {
    if !ctx.bc {
        return Err("the adapter has no BC texture compression".into());
    }
    let root = ac::root().ok_or_else(|| eo_data::external::CC0.looked_in())?;
    ac::available(&root).map_err(|e| format!("{} ({e:#})", root.display()))
}

/// Said once per process.
pub fn say_unavailable(e: &str) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        eprintln!("cc0: the CC0 texture sets are not used: {e}. The renderer keeps its own materials (fetch them with `python3 scripts/setup-assets.py --only cc0`, or point EO_CC0_ROOT at a copy).")
    });
}

fn srgb_byte_to_linear(b: u8) -> f32 {
    let c = b as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// The kit map layers' mean linear albedo (each layer's last mip, averaged in linear light when it was made).
pub fn kit_means(maps: &eo_render::KitUpload) -> Vec<[f32; 3]> {
    maps.layers
        .iter()
        .map(|l| match l[0].last() {
            Some(px) if px.len() >= 3 => [srgb_byte_to_linear(px[0]), srgb_byte_to_linear(px[1]), srgb_byte_to_linear(px[2])],
            _ => [0.5; 3],
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct Cc0Stats {
    pub sets: usize,
    pub from_cache: usize,
    pub cooked: usize,
    pub longest_set_ms: f64,
    pub first_set_at_ms: Option<f64>,
    pub all_received_at_ms: Option<f64>,
    pub ready_at_ms: Option<f64>,
    pub failed: Vec<String>,
    pub gpu_block_bytes: u64,
    pub materials_mapped: usize,
}

type Arrival = (ac::SetKind, u32, Result<ac::CookedSet, String>);

pub struct Cc0Runtime {
    rx: Receiver<Arrival>,
    total: usize,
    received: usize,
    uploaded: usize,
    in_queue: usize,
    mats: Vec<rc::LayerInfo>,
    decals: Vec<rc::LayerInfo>,
    finished: bool,
    failed: bool,
    started: Instant,
    pub stats: Cc0Stats,
}

impl Cc0Runtime {
    pub fn start(cache: Option<std::path::PathBuf>, started: Instant) -> Self {
        let sets = Arc::new(ac::all_sets());
        let total = sets.len();
        let (tx, rx) = std::sync::mpsc::sync_channel::<Arrival>(2);
        let next = Arc::new(AtomicUsize::new(0));
        let root = ac::root().unwrap_or_default();
        // A few threads: from the cache a set is a read and a digest (0.1-0.2 s), and
        // only two are queued at a time; more only crowded the boot (a 20 ms pump
        // was measured with twelve) without bringing the sets in sooner. (A first cook,
        // without the cache, takes a few minutes on six.)
        let threads = (std::thread::available_parallelism().map(|p| p.get()).unwrap_or(4) / 4).clamp(2, 6);
        for _ in 0..threads {
            let (tx, next, sets, root, cache) = (tx.clone(), next.clone(), sets.clone(), root.clone(), cache.clone());
            std::thread::Builder::new()
                .name("eo-cc0-cook".into())
                .spawn(move || {
                    eo_render::hitch::worker_started(); // below the render thread
                    loop {
                        let i = next.fetch_add(1, Ordering::SeqCst);
                        let Some((kind, layer, def)) = sets.get(i).copied() else { break };
                        let r = ac::load_cached(&root, &def, cache.as_deref()).map_err(|e| format!("{}: {e:#}", def.id));
                        if tx.send((kind, layer, r)).is_err() {
                            break;
                        }
                    }
                })
                .expect("spawn cc0 cook thread");
        }
        Self {
            rx,
            total,
            received: 0,
            uploaded: 0,
            in_queue: 0,
            mats: vec![Default::default(); ac::MATERIAL_SETS.len()],
            decals: vec![Default::default(); ac::DECAL_SETS.len()],
            finished: false,
            failed: false,
            started,
            stats: Cc0Stats { sets: total, ..Default::default() },
        }
    }

    fn at_ms(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1e3
    }

    /// Queue arrived sets (two in the queue at most); write the tables once all are on the GPU.
    pub fn pump(&mut self, renderer: &Renderer, queue: &wgpu::Queue, uploads: &mut UploadQueue, materials: &MaterialTable, kit_means: Option<&[[f32; 3]]>) {
        if self.finished || self.failed {
            return;
        }
        while self.in_queue < 2 {
            let Ok((kind, layer, r)) = self.rx.try_recv() else { break };
            self.received += 1;
            if self.stats.first_set_at_ms.is_none() {
                self.stats.first_set_at_ms = Some(self.at_ms());
            }
            if self.received == self.total {
                self.stats.all_received_at_ms = Some(self.at_ms());
            }
            match r {
                Ok(mut set) => {
                    if set.from_cache {
                        self.stats.from_cache += 1;
                    } else {
                        self.stats.cooked += 1;
                    }
                    self.stats.longest_set_ms = self.stats.longest_set_ms.max(set.ms);
                    self.stats.gpu_block_bytes += set.bytes();
                    let info = rc::LayerInfo::from_set(&set);
                    match kind {
                        ac::SetKind::Material => self.mats[layer as usize] = info,
                        ac::SetKind::Decal => self.decals[layer as usize] = info,
                    }
                    let idx = match kind {
                        ac::SetKind::Material => layer as u64,
                        ac::SetKind::Decal => 64 + layer as u64,
                    };
                    let writes = rc::set_writes(kind, layer, &mut set);
                    uploads.push(Job::new(TAG_CC0 | idx, writes, vec![]));
                    self.in_queue += 1;
                }
                Err(e) => {
                    eprintln!("cc0: {e}; the CC0 sets stay off and the renderer keeps its own materials");
                    self.stats.failed.push(e);
                    self.failed = true;
                    return;
                }
            }
        }
        if self.uploaded == self.total {
            if let Some(km) = kit_means {
                let (mats, mapped) = rc::material_table(materials, km, &self.mats);
                let layers = rc::layer_table(&self.mats, &self.decals);
                renderer.cc0_finish(queue, &mats, &layers);
                self.stats.materials_mapped = mapped;
                self.stats.ready_at_ms = Some(self.at_ms());
                self.finished = true;
                eprintln!(
                    "cc0: {} sets on the GPU ({} from the cache, {} cooked; {:.1} MB of BC blocks), {mapped} materials draw a scan; ready {:.0} ms after process start",
                    self.total,
                    self.stats.from_cache,
                    self.stats.cooked,
                    self.stats.gpu_block_bytes as f64 / 1e6,
                    self.at_ms()
                );
            }
        }
    }

    pub fn after_pump(&mut self, done: &[u64]) {
        for &t in done {
            if t & TAG_CC0 != 0 && t >> 58 == 0 {
                self.uploaded += 1;
                self.in_queue = self.in_queue.saturating_sub(1);
            }
        }
    }

    /// Every set on the GPU and the tables written (or given up).
    pub fn settled(&self) -> bool {
        self.finished || self.failed
    }

    pub fn report(&self) -> serde_json::Value {
        let s = &self.stats;
        serde_json::json!({
            "sets": s.sets, "from_cache": s.from_cache, "cooked": s.cooked, "longest_set_ms": s.longest_set_ms,
            "process_start_to_first_set_ms": s.first_set_at_ms, "process_start_to_all_sets_ms": s.all_received_at_ms,
            "process_start_to_on_ms": s.ready_at_ms, "failed": s.failed, "gpu_block_bytes": s.gpu_block_bytes,
            "materials_mapped": s.materials_mapped, "on": self.finished,
        })
    }
}
