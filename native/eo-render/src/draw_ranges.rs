//! Conservative draw ranges for the fixed-slot draw lists (an adapter
//! without multi-draw-indirect-count).
//!
//! Without `MULTI_DRAW_INDIRECT_COUNT` every object keeps its own slot in each
//! list (the camera's opaque and transparent lists, each shadow cascade's), the
//! cull passes write instance count 0 or 1 into it, and the draw issues one
//! indirect draw per slot: wgpu's Metal backend loops them one call each and
//! its indirect-draw validation adds a per-draw record and a GPU copy, about
//! 40 ns of render-thread time per slot. With 213 k objects that was 26 ms of
//! `CommandEncoder::finish` per frame on three lists, for about 400 objects
//! drawn.
//!
//! The CPU keeps a copy of the object table (every write the GPU table gets is
//! applied to it, see `Renderer::write` and the upload stager) and, per block
//! of [`BLOCK`] consecutive slots and per list, the bounds of the objects of
//! the block that the list's cull could draw at all: the camera lists leave
//! out empty objects, the other list's objects (blend bit), the flat sea quad
//! while the water pass is on, and kit footprint boxes whose far level is
//! resident (cull.wgsl `lod`: such a group's level is never the box); the
//! shadow lists leave out what `look_shadow_cull.wgsl` never casts (ground
//! kinds, glass, terrain, open ground, a landmark's far item). Each frame a
//! block is drawn only if those bounds meet the frustum (or the cascade box)
//! by the cull's own test with a margin. The cull passes still decide every
//! slot, and the ranges keep slot order, so every frame draws exactly what it
//! drew before; only slots no cull can fill are no longer issued.
//!
//! A block that was written in the last [`VOLATILE_FRAMES`] frames is drawn
//! whole (the GPU copy of a streamed write may land a frame after the CPU
//! copy changed). A stress-multiplied table (`--multiply`) keeps the full lists.

use crate::scene::{BLEND_BIT, GpuObject, LOD_KIT_BOX, LOD_LANDMARK_FAR, LOD_LANDMARK_NEAR, LOD_TERRAIN, NO_GROUP};

/// Slots per block.
pub const BLOCK: usize = 64;
/// A written block is drawn whole for this many frames.
pub const VOLATILE_FRAMES: u32 = 8;
/// Metres of slack on each plane test (the GPU test is the same in f32).
const MARGIN_M: f32 = 0.05;

/// The lists a block summarises.
const CAM_OPAQUE: usize = 0;
const CAM_BLEND: usize = 1;
const SHADOW: usize = 2;
/// the sea quad's own bounds (camera opaque list, only with the water pass off).
const CAM_SEA: usize = 3;
const LISTS: usize = 4;

/// One list's draw: object (slot) ranges, ascending, disjoint.
pub type Ranges = Vec<std::ops::Range<u32>>;

#[derive(Clone, Copy, Debug)]
struct Bounds {
    min: [f32; 3],
    max: [f32; 3],
}

impl Bounds {
    const EMPTY: Bounds = Bounds { min: [f32::INFINITY; 3], max: [f32::NEG_INFINITY; 3] };
    fn is_empty(&self) -> bool {
        self.min[0] > self.max[0]
    }
    fn add(&mut self, o: &GpuObject) {
        for k in 0..3 {
            self.min[k] = self.min[k].min(o.min[k]);
            self.max[k] = self.max[k].max(o.max[k]);
        }
    }
    /// cull.wgsl's AABB test (positive vertex per plane), with `MARGIN_M`.
    fn meets(&self, planes: &[[f32; 4]]) -> bool {
        if self.is_empty() {
            return false;
        }
        planes.iter().all(|p| {
            let v = [
                if p[0] >= 0.0 { self.max[0] } else { self.min[0] },
                if p[1] >= 0.0 { self.max[1] } else { self.min[1] },
                if p[2] >= 0.0 { self.max[2] } else { self.min[2] },
            ];
            let d = p[0] * v[0] + p[1] * v[1] + p[2] * v[2] + p[3];
            // non-finite bounds or planes: keep the block
            !(d < -MARGIN_M) || !d.is_finite()
        })
    }
}

#[derive(Clone, Copy)]
struct Block {
    lists: [Bounds; LISTS],
    dirty: bool,
    /// the frame (`DrawRanges::frame`) the block was last written in.
    written: u32,
}

/// The CPU copy of the object table and the per-block summaries.
pub struct DrawRanges {
    objects: Vec<GpuObject>,
    blocks: Vec<Block>,
    /// per LOD group its far cell (`Renderer::write_hlod_map`), per cell its residency (`write_hlod_flags`).
    hlod_map: Vec<u32>,
    hlod_flags: Vec<u32>,
    hlod_on: bool,
    frame: u32,
    /// counters for the bench: slots issued and slots in the lists, last frame.
    pub issued: u64,
    pub slots: u64,
}

impl DrawRanges {
    pub fn new(objects: &[GpuObject], hlod_on: bool) -> Self {
        let n = objects.len();
        let blocks = vec![Block { lists: [Bounds::EMPTY; LISTS], dirty: true, written: 0 }; n.div_ceil(BLOCK)];
        Self { objects: objects.to_vec(), blocks, hlod_map: vec![], hlod_flags: vec![], hlod_on, frame: VOLATILE_FRAMES + 1, issued: 0, slots: 0 }
    }

    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Apply a write of `data` at byte `offset` of the object table (any
    /// alignment; bytes past the table are ignored).
    pub fn write(&mut self, offset: u64, data: &[u8]) {
        let bytes: &mut [u8] = bytemuck::cast_slice_mut(&mut self.objects);
        let start = (offset as usize).min(bytes.len());
        let end = (offset as usize).saturating_add(data.len()).min(bytes.len());
        if start >= end {
            return;
        }
        bytes[start..end].copy_from_slice(&data[..end - start]);
        let os = std::mem::size_of::<GpuObject>();
        let (o0, o1) = (start / os, (end - 1) / os);
        for b in o0 / BLOCK..=o1 / BLOCK {
            self.blocks[b].dirty = true;
            self.blocks[b].written = self.frame;
        }
    }

    pub fn set_hlod_map(&mut self, map: &[u32]) {
        if self.hlod_map != map {
            self.hlod_map = map.to_vec();
            self.mark_boxes_dirty();
        }
    }

    pub fn set_hlod_flags(&mut self, flags: &[u32]) {
        if self.hlod_flags != flags {
            self.hlod_flags = flags.to_vec();
            self.mark_boxes_dirty();
        }
    }

    fn mark_boxes_dirty(&mut self) {
        for (b, blk) in self.blocks.iter_mut().enumerate() {
            let lo = b * BLOCK;
            let hi = (lo + BLOCK).min(self.objects.len());
            if self.objects[lo..hi].iter().any(|o| o.lod_role == LOD_KIT_BOX) {
                blk.dirty = true;
            }
        }
    }

    /// cull.wgsl `lod`: a kit group whose cell or district is resident is
    /// never drawn at the box level (its level is the far layer's or a rung).
    fn box_stands_down(&self, group: u32) -> bool {
        if !self.hlod_on || group == NO_GROUP {
            return false;
        }
        let Some(&cell) = self.hlod_map.get(group as usize) else { return false };
        if cell == u32::MAX {
            return false;
        }
        self.hlod_flags.get(cell as usize).is_some_and(|f| f & 3 != 0)
    }

    fn rebuild(&mut self, b: usize) {
        let lo = b * BLOCK;
        let hi = (lo + BLOCK).min(self.objects.len());
        let mut lists = [Bounds::EMPTY; LISTS];
        for o in &self.objects[lo..hi] {
            let kind = o.kind & 255;
            let blend = o.kind & BLEND_BIT != 0;
            // cull.wgsl `lod_and_frustum`: a terrain tile may draw its coarse range.
            let any = o.index_count > 0 || (o.lod_role == LOD_TERRAIN && o.far_index_count > 0);
            if any {
                let boxed = o.lod_role == LOD_KIT_BOX && self.box_stands_down(o.lod_group);
                if !boxed {
                    if blend {
                        lists[CAM_BLEND].add(o);
                    } else if kind == crate::scene::ObjectKind::Sea as u32 {
                        lists[CAM_SEA].add(o);
                    } else {
                        lists[CAM_OPAQUE].add(o);
                    }
                }
            }
            // look_shadow_cull.wgsl `base_caster` and its role rules (a superset of what casts).
            let ground = matches!(kind, 1 | 2 | 3 | 6 | 7 | 10);
            let landmark_never = (o.lod_role == LOD_LANDMARK_NEAR || o.lod_role == LOD_LANDMARK_FAR) && !(kind == 9 && o.lod_role == LOD_LANDMARK_NEAR);
            if !blend && !ground && o.index_count > 0 && o.lod_role != LOD_TERRAIN && !landmark_never {
                lists[SHADOW].add(o);
            }
        }
        self.blocks[b].lists = lists;
        self.blocks[b].dirty = false;
    }

    /// Start a frame: bring the dirty blocks up to date.
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        for b in 0..self.blocks.len() {
            if self.blocks[b].dirty {
                self.rebuild(b);
            }
        }
        self.issued = 0;
        self.slots = 0;
    }

    fn volatile(&self, b: usize) -> bool {
        self.frame.wrapping_sub(self.blocks[b].written) < VOLATILE_FRAMES
    }

    /// object ranges of one list whose blocks meet `planes`.
    fn ranges(&mut self, lists: &[usize], planes: &[[f32; 4]]) -> Ranges {
        let n = self.objects.len() as u32;
        let mut out: Ranges = Vec::new();
        for b in 0..self.blocks.len() {
            let blk = &self.blocks[b];
            let keep = self.volatile(b) || lists.iter().any(|&l| blk.lists[l].meets(planes));
            if !keep {
                continue;
            }
            let lo = (b * BLOCK) as u32;
            let hi = (lo + BLOCK as u32).min(n);
            match out.last_mut() {
                Some(r) if r.end == lo => r.end = hi,
                _ => out.push(lo..hi),
            }
        }
        self.issued += out.iter().map(|r| (r.end - r.start) as u64).sum::<u64>();
        self.slots += n as u64;
        out
    }

    /// The camera's opaque list (`sea`: the flat sea quad may draw, i.e. the water pass is off).
    pub fn camera_opaque(&mut self, planes: &[[f32; 4]; 6], sea: bool) -> Ranges {
        if sea { self.ranges(&[CAM_OPAQUE, CAM_SEA], planes) } else { self.ranges(&[CAM_OPAQUE], planes) }
    }

    /// The camera's transparent list, as object ranges.
    pub fn camera_blend(&mut self, planes: &[[f32; 4]; 6]) -> Ranges {
        self.ranges(&[CAM_BLEND], planes)
    }

    /// One shadow cascade's list.
    pub fn shadow(&mut self, planes: &[[f32; 4]]) -> Ranges {
        self.ranges(&[SHADOW], planes)
    }
}

/// `ranges` (object indices) as slot ranges of a list whose slot 0 is object
/// `first` (the fallback's transparent list starts at `blend_first`).
pub fn shift(ranges: &[std::ops::Range<u32>], first: u32) -> Ranges {
    ranges.iter().filter(|r| r.end > first).map(|r| r.start.max(first) - first..r.end - first).collect()
}

/// Issue `ranges` of a fixed-slot list at byte `base` of `buf` (one indirect
/// multi-draw per range).
pub fn draw(pass: &mut wgpu::RenderPass<'_>, buf: &wgpu::Buffer, base: u64, ranges: &[std::ops::Range<u32>]) {
    for r in ranges {
        if r.end > r.start {
            pass.multi_draw_indexed_indirect(buf, base + r.start as u64 * crate::DRAW_ARGS_BYTES, r.end - r.start);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    fn obj(x: f32, kind: u32, role: u32, group: u32) -> GpuObject {
        GpuObject { min: [x, 0.0, 0.0], max: [x + 1.0, 1.0, 1.0], index_count: 6, kind, lod_role: role, lod_group: group, ..GpuObject::zeroed() }
    }

    /// planes keeping x in [a, b].
    fn slab(a: f32, b: f32) -> [[f32; 4]; 6] {
        [[1.0, 0.0, 0.0, -a], [-1.0, 0.0, 0.0, b], [0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0]]
    }

    #[test]
    fn blocks_outside_the_frustum_are_skipped_in_order() {
        let objs: Vec<GpuObject> = (0..BLOCK * 4).map(|i| obj((i / BLOCK) as f32 * 100.0, 0, 0, NO_GROUP)).collect();
        let mut r = DrawRanges::new(&objs, true);
        for _ in 0..VOLATILE_FRAMES + 1 {
            r.begin_frame();
        }
        let got = r.camera_opaque(&slab(150.0, 350.0), false);
        assert_eq!(got, vec![(2 * BLOCK) as u32..(4 * BLOCK) as u32]);
        assert!(r.camera_blend(&slab(-1e9, 1e9)).is_empty(), "no glass: nothing in the transparent list");
    }

    #[test]
    fn a_resident_far_level_retires_the_kit_boxes_and_a_write_keeps_its_block() {
        let objs: Vec<GpuObject> = (0..BLOCK * 2).map(|i| obj(0.0, 0, LOD_KIT_BOX, (i / BLOCK) as u32)).collect();
        let mut r = DrawRanges::new(&objs, true);
        r.set_hlod_map(&[0, 1]);
        r.set_hlod_flags(&[2, 0]); // group 0's district resident, group 1's nothing
        for _ in 0..VOLATILE_FRAMES + 1 {
            r.begin_frame();
        }
        let all = slab(-1e9, 1e9);
        assert_eq!(r.camera_opaque(&all, false), vec![BLOCK as u32..(2 * BLOCK) as u32]);
        // the boxes still cast in the far cascade
        assert_eq!(r.shadow(&all), vec![0..(2 * BLOCK) as u32]);
        // a write into block 0 draws it whole for a while
        let o = obj(5.0, 0, LOD_KIT_BOX, 0);
        r.write(3 * std::mem::size_of::<GpuObject>() as u64, bytemuck::bytes_of(&o));
        r.begin_frame();
        assert_eq!(r.camera_opaque(&all, false), vec![0..(2 * BLOCK) as u32]);
        // the far level evicted: the boxes are back
        r.set_hlod_flags(&[0, 0]);
        for _ in 0..VOLATILE_FRAMES + 1 {
            r.begin_frame();
        }
        assert_eq!(r.camera_opaque(&all, false), vec![0..(2 * BLOCK) as u32]);
    }

    #[test]
    fn the_transparent_list_and_the_sea_follow_their_rules() {
        let mut objs: Vec<GpuObject> = (0..BLOCK * 3).map(|_| obj(0.0, 0, 0, NO_GROUP)).collect();
        objs[BLOCK + 5].kind = BLEND_BIT; // glass in block 1
        for o in &mut objs[2 * BLOCK..] {
            o.kind = crate::scene::ObjectKind::Sea as u32;
        }
        for o in &mut objs[..BLOCK] {
            o.index_count = 0; // empty slots
        }
        let mut r = DrawRanges::new(&objs, false);
        for _ in 0..VOLATILE_FRAMES + 1 {
            r.begin_frame();
        }
        let all = slab(-1e9, 1e9);
        assert_eq!(r.camera_blend(&all), vec![BLOCK as u32..(2 * BLOCK) as u32]);
        assert_eq!(r.camera_opaque(&all, false), vec![BLOCK as u32..(2 * BLOCK) as u32]);
        assert_eq!(r.camera_opaque(&all, true), vec![BLOCK as u32..(3 * BLOCK) as u32]);
        assert_eq!(shift(&[BLOCK as u32..(2 * BLOCK) as u32], BLOCK as u32 + 10), vec![0..(BLOCK - 10) as u32]);
    }

    /// On the GPU (the fixed-slot fallback): every slot the cull pass fills
    /// lies in this frame's ranges, over a scattered city and many cameras.
    #[test]
    fn every_slot_the_gpu_cull_fills_is_in_a_range() {
        use crate::camera::Camera;
        use crate::scene::{ObjectKind, Vertex};
        let opts = crate::GpuOptions { force_no_indirect_count: true, force_no_count_emulation: true, ..Default::default() };
        let ctx = crate::GpuContext::new(crate::GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required for the draw-range test");
        let mut verts = vec![];
        let mut objects = vec![];
        let mut seed = 0x2545_f491_u32;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32
        };
        for i in 0..3000u32 {
            // clusters of neighbours (as the tile-sorted table is), spread over 4 km
            let c = glam::Vec3::new(((i / 40) as f32 * 97.0) % 4000.0 - 2000.0 + rnd() * 60.0, rnd() * 80.0, ((i / 40) as f32 * 61.0) % 4000.0 - 2000.0 + rnd() * 60.0);
            let h = glam::Vec3::splat(1.0 + rnd() * 20.0);
            let base = verts.len() as i32;
            for k in 0..8 {
                let p = c + glam::Vec3::new((k & 1) as f32, ((k >> 1) & 1) as f32, ((k >> 2) & 1) as f32) * 2.0 * h - h;
                verts.push(Vertex { pos: p.to_array(), normal: 0, color: 0xffff_ffff });
            }
            let kind = if i % 97 == 5 { ObjectKind::Sea } else { ObjectKind::Building };
            let mut o = GpuObject::new(kind, (c - h).to_array(), (c + h).to_array(), 0, if i % 53 == 7 { 0 } else { 36 }, base);
            if i % 31 == 3 {
                o.kind |= BLEND_BIT;
            }
            objects.push(o);
        }
        let indices: Vec<u32> = (0..36).map(|i| [0, 1, 2, 1, 3, 2][i % 6] + (i / 6) as u32 % 2 * 4).collect();
        let r = crate::Renderer::from_parts(&ctx, &verts, &indices, &objects, wgpu::TextureFormat::Rgba8UnormSrgb, 1).unwrap();
        assert!(!r.compact && r.ranges.is_some(), "the test needs the fixed-slot lists with draw ranges");
        let (mut checked, mut saved) = (0, 0u64);
        for k in 0..40 {
            let eye = glam::Vec3::new(rnd() * 3000.0 - 1500.0, 2.0 + rnd() * 400.0, rnd() * 3000.0 - 1500.0);
            let cam = Camera::new(eye, rnd() * std::f32::consts::TAU, -rnd() * 0.8, 16.0 / 9.0);
            let out = r.cull_readback(&ctx, &cam).unwrap();
            let fr = r.frame_ranges.lock().unwrap().clone().expect("ranges this frame");
            let inside = |rs: &Ranges, slot: u32| rs.iter().any(|x| x.contains(&slot));
            let (issued, slots) = r.draw_range_stats().unwrap();
            saved += slots - issued;
            for (slot, d) in out.draws.iter().enumerate() {
                if d[1] == 1 {
                    assert!(inside(&fr.0, slot as u32), "camera {k}: opaque slot {slot} drawn by the cull is outside the ranges");
                    checked += 1;
                }
            }
            for (slot, d) in out.blend_draws.iter().enumerate() {
                if d[1] == 1 {
                    assert!(inside(&fr.1, slot as u32), "camera {k}: transparent slot {slot} drawn by the cull is outside the ranges");
                    checked += 1;
                }
            }
        }
        assert!(checked > 100, "the cameras saw too little to test ({checked} draws)");
        assert!(saved > 0, "the ranges never left a block out");
    }
}
