//! the upload queue's staged path (persistent mapped chunks filled by
//! the copier thread, GPU copies submitted a pump later) writes exactly what
//! the direct path (`queue.write_buffer`) writes; a job cancelled while its
//! pieces are being copied never lands over a later job that reuses its
//! range; no pump writes more than the budget; a job finishes only after its
//! bytes are on their way to the GPU; texture mips go in row bands under the
//! same budget and land texel for texel. Needs a GPU.

use eo_render::scene::{GpuObject, ObjectKind, Vertex};
use eo_render::upload::{Data, Job, Target, UploadQueue, Write};
use eo_render::{GpuContext, GpuOptions, Renderer, RendererInit, authored::GpuMaterial, wgpu};

fn ctx() -> GpuContext {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts).expect("a GPU adapter is required for the upload test")
}

/// Room for the test's writes in the object table (it is copy-readable): 16 MiB.
const ARENA_BYTES: usize = 16 << 20;

fn renderer(ctx: &GpuContext) -> Renderer {
    renderer_with_kit(ctx, 0, 1)
}

fn renderer_with_kit(ctx: &GpuContext, kit_layers: u32, kit_res: u32) -> Renderer {
    let verts = vec![Vertex { pos: [0.0; 3], normal: 0, color: 0 }; 3];
    let indices = vec![0u32, 1, 2];
    // Empty slots (nothing drawn) make the room; slot 0 is the one triangle.
    let mut objects = vec![GpuObject::new(ObjectKind::Building, [0.0; 3], [0.0; 3], 0, 0, 0); ARENA_BYTES / std::mem::size_of::<GpuObject>()];
    objects[0] = GpuObject::new(ObjectKind::Building, [0.0; 3], [1.0; 3], 0, 3, 0);
    let materials = vec![GpuMaterial::vertex_colour()];
    Renderer::from_init(
        ctx,
        &RendererInit {
            vertices: &verts,
            indices: &indices,
            objects: &objects,
            extra_vertices: 0,
            extra_indices: 0,
            materials: &materials,
            groups: &[],
            hlod_enter_m: 0.0,
            hlod_exit_m: 0.0,
            kit_layers,
            kit_res,
            color_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            sample_count: 1,
            multiplier: 1,
            debug_ids: false,
            blend_first_object: 0,
        },
    )
    .unwrap()
}

fn pattern(seed: u32, words: usize) -> Vec<u32> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    (0..words)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        })
        .collect()
}

/// A job writing `data` at word `at` of the object table, in `pieces` bulk writes.
fn job(tag: u64, at: u32, data: &[u32], pieces: usize) -> Job {
    let n = data.len().div_ceil(pieces);
    let bulk = data.chunks(n).enumerate().map(|(k, c)| Write { target: Target::Object, offset: (at as u64 + (k * n) as u64) * 4, data: Data::Words(c.to_vec()) }).collect();
    Job::new(tag, bulk, vec![])
}

const BUDGET: u64 = 4 << 20;

/// Pump frames (as the bench does: pump, submit, poll) until `want` jobs finished.
fn run(ctx: &GpuContext, r: &Renderer, q: &mut UploadQueue, want: usize, done: &mut Vec<u64>) {
    run_within(ctx, r, q, want, done, BUDGET)
}

fn run_within(ctx: &GpuContext, r: &Renderer, q: &mut UploadQueue, want: usize, done: &mut Vec<u64>, budget: u64) {
    for _ in 0..10_000 {
        let (st, d) = q.pump(r, &ctx.queue);
        assert!(st.bytes <= budget, "a pump wrote {} B, over the {budget} B budget", st.bytes);
        done.extend(d);
        ctx.queue.submit([]);
        let _ = ctx.device.poll(wgpu::PollType::Poll);
        if done.len() >= want && q.pending() == 0 {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("uploads did not finish ({} of {want} jobs)", done.len());
}

fn read(ctx: &GpuContext, r: &Renderer, at: u32, words: usize) -> Vec<u32> {
    bytemuck::pod_collect_to_vec(&r.buffer_readback(ctx, Target::Object, at as u64 * 4, words as u64 * 4).unwrap())
}

#[test]
fn staged_uploads_equal_direct_writes_and_cancellation_never_lands() {
    let ctx = ctx();
    for staged in [true, false] {
        let r = renderer(&ctx);
        let mut q = UploadQueue::new(BUDGET).with_ring(staged);
        // Two jobs: 3 MiB in five writes (several pieces, more than one chunk
        // with the budget's finish room), and 100 KB.
        let a = pattern(1, 768 * 1024);
        let b = pattern(2, 25 * 1024);
        let (at_a, at_b) = (16, 1_000_000);
        q.push(job(1, at_a, &a, 5));
        q.push(job(2, at_b, &b, 1));
        let mut done = vec![];
        run(&ctx, &r, &mut q, 2, &mut done);
        assert_eq!(done, vec![1, 2], "jobs finish in order (staged {staged})");
        assert!(read(&ctx, &r, at_a, a.len()) == a, "job 1's bytes (staged {staged})");
        assert!(read(&ctx, &r, at_b, b.len()) == b, "job 2's bytes (staged {staged})");
        // Cancellation: job 3 is staged (its pieces in flight), cancelled, and
        // job 4 writes the same range; only job 4's bytes may remain.
        // (6 MiB, more than a pump's budget: still queued on the direct path too)
        let c = pattern(3, 1536 * 1024);
        let d = pattern(4, 1536 * 1024);
        let at_c = 2_000_000;
        q.push(job(3, at_c, &c, 3));
        let _ = q.pump(&r, &ctx.queue);
        let dropped = q.cancel(|t| t != 3);
        assert_eq!(dropped, vec![3], "job 3 was cancelled while in flight (staged {staged})");
        q.push(job(4, at_c, &d, 3));
        let mut done = vec![];
        run(&ctx, &r, &mut q, 1, &mut done);
        assert_eq!(done, vec![4]);
        // A few more frames: nothing of job 3 lands later.
        for _ in 0..8 {
            let _ = q.pump(&r, &ctx.queue);
            ctx.queue.submit([]);
            let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        }
        assert!(read(&ctx, &r, at_c, d.len()) == d, "a cancelled job's staged bytes landed over its successor (staged {staged})");
        if staged {
            let (ready, chunks, grew, bytes) = q.stager_stats().unwrap();
            assert!(bytes >= ((a.len() + b.len() + d.len()) * 4) as u64, "the staged path carried the bytes ({bytes} B)");
            assert!(grew == 0, "the pre-made chunks sufficed (grew {grew})");
            assert!(ready == chunks, "every chunk came back mapped ({ready} of {chunks})");
        }
    }
}

/// Texture mips go through the same budget: a mip larger than a pump's
/// budget goes in bands of whole rows over several pumps (rows narrower than
/// the copy's 256-byte alignment included), on both paths, and every texel
/// lands where a whole-mip `write_texture` would have put it.
#[test]
fn texture_mips_go_in_row_bands_within_the_budget() {
    let ctx = ctx();
    let (layers, res) = (2u32, 256u32);
    let mips = res.ilog2() + 1;
    // 40 KiB: a 256-texel mip (256 KiB) takes seven pumps, and bands end mid-mip.
    let budget = 40 << 10;
    for staged in [true, false] {
        let r = renderer_with_kit(&ctx, layers, res);
        let mut q = UploadQueue::new(budget).with_ring(staged);
        let mut want = vec![];
        let mut bulk = vec![];
        for layer in 0..layers {
            for map in 0..3u8 {
                for mip in 0..mips {
                    let side = (res >> mip) as usize;
                    let target = Target::Texture { map, layer, mip };
                    let words = pattern(1 + layer * 100 + map as u32 * 10 + mip, side * side);
                    bulk.push(Write { target, offset: 0, data: Data::Bytes(bytemuck::cast_slice(&words).to_vec()) });
                    want.push((target, words));
                }
            }
        }
        q.push(Job::new(7, bulk, vec![]));
        let mut done = vec![];
        run_within(&ctx, &r, &mut q, 1, &mut done, budget);
        assert_eq!(done, vec![7]);
        for (target, words) in want {
            let got: Vec<u32> = bytemuck::pod_collect_to_vec(&r.texture_readback(&ctx, target).unwrap());
            assert!(got == words, "{target:?} (staged {staged})");
        }
        if staged {
            let (_, _, _, bytes) = q.stager_stats().unwrap();
            let total: u64 = (0..mips).map(|m| ((res >> m) as u64).pow(2) * 4).sum::<u64>() * 3 * layers as u64;
            assert!(bytes == total, "the stager carried every texel ({bytes} of {total} B)");
        }
    }
}

/// the CC0 arrays' BC mips go through the stager too, in bands of whole
/// block rows (16-byte BC7 / BC5 blocks, four texels a row) under the same
/// byte budget, a texture piece starting on the chunk's texture alignment
/// after a buffer piece that left the chunk unaligned, on both paths; every
/// block lands where a whole-mip write would have put it, and the decal
/// grid's buffer write lands beside them.
#[test]
fn cc0_bc_mips_go_in_block_row_bands_within_the_budget() {
    let ctx = ctx();
    if !ctx.bc {
        eprintln!("cc0: the adapter samples no BC; the CC0 arrays are never allocated here");
        return;
    }
    let budget = 40 << 10;
    for staged in [true, false] {
        let r = renderer(&ctx);
        assert!(r.attach_cc0(&ctx) > 0, "the CC0 arrays allocate");
        let mut q = UploadQueue::new(budget).with_ring(staged);
        let grid = pattern(99, 6);
        let mut bulk = vec![Write { target: Target::Cc0Grid, offset: 16, data: Data::Words(grid.clone()) }];
        let mut want = vec![];
        // A material layer's three maps (2048 px) and a decal layer's albedo (1024 px), every mip down to one block.
        for (array, layer) in [(0u8, 3u16), (1, 3), (2, 3), (3, 1)] {
            let mips = r.upload_texture(Target::Cc0Texture { array, layer, mip: 0 }).unwrap().texture.mip_level_count();
            for mip in 0..mips {
                let target = Target::Cc0Texture { array, layer, mip: mip as u8 };
                let n = r.upload_texture(target).unwrap().bytes() as usize;
                let bytes: Vec<u8> = bytemuck::cast_slice(&pattern(7 + array as u32 * 31 + mip, n / 4)).to_vec();
                bulk.push(Write { target, offset: 0, data: Data::Bytes(bytes.clone()) });
                want.push((target, bytes));
            }
        }
        q.push(Job::new(9, bulk, vec![]));
        let mut done = vec![];
        run_within(&ctx, &r, &mut q, 1, &mut done, budget);
        assert_eq!(done, vec![9]);
        for (target, bytes) in want {
            assert!(r.texture_readback(&ctx, target).unwrap() == bytes, "{target:?} (staged {staged})");
        }
        let got: Vec<u32> = bytemuck::pod_collect_to_vec(&r.buffer_readback(&ctx, Target::Cc0Grid, 16, 24).unwrap());
        assert_eq!(got, grid, "the decal grid write (staged {staged})");
    }
}
