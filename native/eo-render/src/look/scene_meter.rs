//! The scene meter on the GPU (`--no-scene-meter`): one compute pass over the
//! frame post reads, binning its luminance (look_meter.wgsl), copied into a
//! ring of readback buffers and read one or more frames late; eo-sky
//! `night::scene_meter` turns a reading into the exposure.

use eo_sky::night::{SCENE_METER_BINS, SCENE_METER_BINS_PER_EV, SCENE_METER_LO_EV, SceneMeterReading};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

const RING: usize = 4;
const HIST_BYTES: u64 = (SCENE_METER_BINS * 4) as u64;

struct Slot {
    buf: wgpu::Buffer,
    /// 0 free, 1 copied (submitted with a frame), 2 mapping, 3 mapped, 4 failed.
    stage: Arc<AtomicU8>,
    prior: f64,
    frame: u32,
}

pub struct SceneMeter {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    hist: wgpu::Buffer,
    slots: Vec<Mutex<Slot>>,
    /// bind groups for the three sources post may read (the HDR scene, history 0, 1), by size.
    binds: Mutex<Option<((u32, u32), [wgpu::BindGroup; 3])>>,
    latest: Mutex<Option<SceneMeterReading>>,
    pub bytes: u64,
}

fn new_slot_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some("scene-meter-read"), size: HIST_BYTES, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
}

pub fn wgsl() -> String {
    format!(
        "const MT_BINS: u32 = {}u;\nconst MT_BINS_PER_EV: f32 = {:.1};\nconst MT_LO_EV: f32 = {:.1};\n{}",
        SCENE_METER_BINS,
        SCENE_METER_BINS_PER_EV,
        SCENE_METER_LO_EV,
        include_str!("../look_meter.wgsl")
    )
}

impl SceneMeter {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("look_meter.wgsl"), source: wgpu::ShaderSource::Wgsl(wgsl().into()) });
        let cs = wgpu::ShaderStages::COMPUTE;
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: cs, ty, count: None };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene-meter-bgl"),
            entries: &[
                entry(0, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                entry(1, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                entry(2, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("scene-meter"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("scene-meter"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("meter_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("scene-meter-u"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let hist = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene-meter-hist"),
            size: HIST_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let slots = (0..RING)
            .map(|_| {
                Mutex::new(Slot {
                    buf: new_slot_buffer(device),
                    stage: Arc::new(AtomicU8::new(0)),
                    prior: 1.0,
                    frame: 0,
                })
            })
            .collect();
        Self { pipeline, bgl, uniform, hist, slots, binds: Mutex::new(None), latest: Mutex::new(None), bytes: HIST_BYTES * (RING as u64 + 1) + 16 }
    }

    /// (Re)make the bind groups for the three post sources of a size.
    pub fn bind_sources(&self, device: &wgpu::Device, size: (u32, u32), sources: [&wgpu::TextureView; 3]) {
        let make = |v: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("scene-meter-bg"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(v) },
                    wgpu::BindGroupEntry { binding: 1, resource: self.uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: self.hist.as_entire_binding() },
                ],
            })
        };
        *self.binds.lock().unwrap() = Some((size, sources.map(make)));
    }

    /// Collect finished readbacks (the newest becomes `latest`) and start mapping
    /// the ones submitted since. Call once a frame before encoding. `wait_for`:
    /// a frame whose reading the exposure is waiting on (the first of a new
    /// view, after a cut): when it is in flight the GPU is waited on, so the
    /// shot's exposure is set from its own frame from the next frame on,
    /// whatever the GPU's lag (deterministic captures and tests).
    pub fn poll(&self, device: &wgpu::Device, wait_for: Option<u32>) {
        let mut waiting = false;
        for s in &self.slots {
            let s = s.lock().unwrap();
            let st = s.stage.load(Ordering::Acquire);
            if st == 1 {
                s.stage.store(2, Ordering::Release);
                let flag = s.stage.clone();
                s.buf.slice(..).map_async(wgpu::MapMode::Read, move |r| flag.store(if r.is_ok() { 3 } else { 4 }, Ordering::Release));
            }
            if (st == 1 || st == 2) && wait_for.is_some_and(|a| s.frame >= a) {
                waiting = true;
            }
        }
        let _ = device.poll(if waiting { wgpu::PollType::wait_indefinitely() } else { wgpu::PollType::Poll });
        self.collect(device);
    }

    /// Make sure the coming frame has a free readback slot when its reading is
    /// needed (`must`: the first frame of a new view): with every slot in flight
    /// (a loop running ahead of the GPU) wait for them. Otherwise a frame with no
    /// free slot is simply not metered (the exposure keeps the last reading), so a
    /// paced frame never waits on the GPU for the meter.
    pub fn ensure_free(&self, device: &wgpu::Device, must: bool) {
        if !must || self.slots.iter().any(|s| s.lock().unwrap().stage.load(Ordering::Acquire) == 0) {
            return;
        }
        for s in &self.slots {
            let s = s.lock().unwrap();
            if s.stage.load(Ordering::Acquire) == 1 {
                s.stage.store(2, Ordering::Release);
                let flag = s.stage.clone();
                s.buf.slice(..).map_async(wgpu::MapMode::Read, move |r| flag.store(if r.is_ok() { 3 } else { 4 }, Ordering::Release));
            }
        }
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        self.collect(device);
    }

    fn collect(&self, device: &wgpu::Device) {
        for s in &self.slots {
            let mut s = s.lock().unwrap();
            match s.stage.load(Ordering::Acquire) {
                3 => {
                    let hist: Vec<u32> = match s.buf.slice(..).get_mapped_range() {
                        Ok(v) => bytemuck::cast_slice::<u8, u32>(&v[..HIST_BYTES as usize]).to_vec(),
                        Err(_) => vec![],
                    };
                    s.buf.unmap();
                    s.stage.store(0, Ordering::Release);
                    if hist.len() == SCENE_METER_BINS {
                        let mut l = self.latest.lock().unwrap();
                        if l.as_ref().is_none_or(|r| r.frame <= s.frame) {
                            *l = Some(SceneMeterReading { hist, prior: s.prior, frame: s.frame });
                        }
                    }
                }
                4 => {
                    // the map failed (the buffer was not mapped, and may no longer be
                    // usable: seen once on the drive bench path): a fresh buffer, no reading
                    s.buf = new_slot_buffer(device);
                    s.stage.store(0, Ordering::Release);
                }
                _ => {}
            }
        }
    }

    /// The prior exposure the coming frame's pass bins about (its grey is 0.18 x 0.6 / prior).
    pub fn set_prior(&self, queue: &wgpu::Queue, prior: f64) {
        let grey = (0.18 * 0.6 / prior.max(1e-12)) as f32;
        // grey.y: 1 = the averaging meter (no centre weighting; eo-sky `night::meter_averaging`,
        // off with `--no-meter-highlights` / `--no-gg3-colour`), 0 = round 2's centre weighting
        let avg = if eo_sky::night::meter_averaging() { 1.0f32 } else { 0.0 };
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&[grey, avg, 0.0, 0.0]));
    }

    pub fn latest(&self) -> Option<SceneMeterReading> {
        self.latest.lock().unwrap().clone()
    }

    /// Meter the frame post reads (`src`: 0 the HDR scene, 1 + p history p),
    /// binned about the prior exposure's grey, tagged with the frame. Skipped
    /// when every readback slot is still in flight.
    /// `ts`: the pass's timestamp pair (bench); an empty pass carries it when nothing is metered.
    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, src: usize, size: (u32, u32), prior: f64, frame: u32, ts: Option<(&wgpu::QuerySet, u32)>) {
        let tw = ts.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b), end_of_pass_write_index: Some(b + 1) });
        let empty = |enc: &mut wgpu::CommandEncoder, tw: Option<wgpu::ComputePassTimestampWrites<'_>>| {
            if tw.is_some() {
                enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("scene-meter (none)"), timestamp_writes: tw });
            }
        };
        let binds = self.binds.lock().unwrap();
        let Some((bsize, bg)) = binds.as_ref() else { return empty(enc, tw) };
        if *bsize != size {
            return empty(enc, tw);
        }
        let Some(slot) = self.slots.iter().find(|s| s.lock().unwrap().stage.load(Ordering::Acquire) == 0) else { return empty(enc, tw) };
        enc.clear_buffer(&self.hist, 0, None);
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("scene-meter"), timestamp_writes: tw });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bg[src.min(2)], &[]);
            let (qx, qy) = (size.0.div_ceil(4), size.1.div_ceil(4));
            pass.dispatch_workgroups(qx.div_ceil(16), qy.div_ceil(16), 1);
        }
        let mut s = slot.lock().unwrap();
        enc.copy_buffer_to_buffer(&self.hist, 0, &s.buf, 0, HIST_BYTES);
        s.prior = prior;
        s.frame = frame;
        s.stage.store(1, Ordering::Release);
    }
}
