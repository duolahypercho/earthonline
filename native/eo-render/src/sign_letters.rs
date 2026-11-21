//! Shop sign lettering on the GPU (stage S1, eo-app `--facade-clutter`): the
//! word cells of the facade clutter's signs (eo-kit `clutter::letter_cells`)
//! drawn from the system's fonts (eo-props `shop_lettering`, no font file
//! shipped, nothing written to disk) into a 2D array texture bound as group 0
//! binding 28 of the city pipelines (draw.wgsl `sgn_atlas`, sampled by
//! `sgn_sign` on the lettered sign faces).
//!
//! The cells are drawn on a worker as soon as the kit city starts loading
//! (`start`, from `kit::spawn_kit_city`), and the renderer waits for them when
//! it is made, so the atlas is there from the first frame (deterministic
//! captures) and the load is not lengthened by much. Without the clutter, or
//! with no font installed, the binding holds a 1 x 1 stand-in (2 bytes, not
//! counted) and draw.wgsl draws W3's band in place of the words.

use eo_props::shop_lettering::{self, CellSpec, LetterAtlas, LetterFonts};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Instant;

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Unorm;

/// What the worker drew.
pub struct Built {
    /// None: no font installed (or the clutter off).
    pub atlas: Option<LetterAtlas>,
    /// The font file each face slot took (bold, condensed, serif, script).
    pub fonts: [Option<PathBuf>; 4],
    pub ms: f64,
}

static JOB: Mutex<Option<JoinHandle<Built>>> = Mutex::new(None);
static DONE: OnceLock<Arc<Built>> = OnceLock::new();

/// Draw every cell with the system's fonts (the worker's job; also callable directly).
pub fn build() -> Built {
    let t = Instant::now();
    let cells = eo_kit::clutter::letter_cells();
    let specs: Vec<CellSpec> = cells.iter().map(|c| CellSpec { text: &c.text, face: c.face, stacked: c.stacked, aspect: c.aspect as f32 }).collect();
    let fonts = LetterFonts::system();
    let files = fonts.as_ref().map(|f| f.files().map(|p| p.map(|p| p.to_path_buf()))).unwrap_or_default();
    let atlas = fonts.as_ref().map(|f| shop_lettering::build(f, &specs));
    Built { atlas, fonts: files, ms: t.elapsed().as_secs_f64() * 1e3 }
}

/// Start drawing the cells on a worker, once, when the facade clutter is on.
pub fn start() {
    if eo_kit::clutter::level() == 0 || DONE.get().is_some() {
        return;
    }
    let mut job = JOB.lock().unwrap();
    if job.is_none() {
        *job = Some(std::thread::spawn(|| {
            crate::hitch::worker_started(); // below the render thread
            build()
        }));
    }
}

/// The drawn cells (waits for the worker; draws them here if none was started).
/// None when the clutter is off.
pub fn built() -> Option<Arc<Built>> {
    if eo_kit::clutter::level() == 0 {
        return None;
    }
    if let Some(d) = DONE.get() {
        return Some(d.clone());
    }
    let job = JOB.lock().unwrap().take();
    let b = match job {
        Some(h) => h.join().unwrap_or_else(|_| Built { atlas: None, fonts: Default::default(), ms: 0.0 }),
        None => build(),
    };
    Some(DONE.get_or_init(|| Arc::new(b)).clone())
}

/// The atlas on the GPU (or the stand-in).
pub struct SignLettersGpu {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// Bytes allocated (0 for the stand-in).
    pub bytes: u64,
    pub layers: u32,
}

impl SignLettersGpu {
    /// The atlas when the facade clutter is on and a font is installed, else the stand-in.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        start();
        match built() {
            Some(b) => {
                if let Some(a) = &b.atlas {
                    let names: Vec<String> = b.fonts.iter().map(|f| f.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "-".into())).collect();
                    eprintln!(
                        "sign lettering: {} cells drawn in {:.0} ms from the system fonts [{}]{}",
                        a.layers,
                        b.ms,
                        names.join(", "),
                        if a.missing.is_empty() { String::new() } else { format!(", {} with no face", a.missing.len()) }
                    );
                    return Self::upload(device, queue, a);
                }
                eprintln!("sign lettering: no font installed (bold, condensed, serif or script; EO_UI_FONT_DIR): the signs keep their emblems");
                Self::stand_in(device, queue)
            }
            None => Self::stand_in(device, queue),
        }
    }

    pub fn stand_in(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sign-letters-stand-in"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &[0, 0],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(2), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        Self { texture, view, bytes: 0, layers: 0 }
    }

    pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, a: &LetterAtlas) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sign-letters"),
            size: wgpu::Extent3d { width: a.width, height: a.height, depth_or_array_layers: a.layers },
            mip_level_count: a.levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        for (m, level) in a.levels.iter().enumerate() {
            let (w, h) = a.level_size(m);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: m as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                level,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 2), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: a.layers },
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        Self { texture, view, bytes: a.bytes(), layers: a.layers }
    }
}

#[cfg(test)]
mod tests {
    /// draw.wgsl's tables are eo-kit's: the fields' paint, the inks, the bleach,
    /// the code layout's constants and the face slots eo-props draws.
    #[test]
    fn the_shader_tables_are_the_planners() {
        use eo_kit::clutter as c;
        let src = include_str!("draw.wgsl");
        let body = |name: &str| -> Vec<f32> {
            let at = src.find(&format!("const {name} = array<vec3<f32>")).unwrap_or_else(|| panic!("{name} in draw.wgsl"));
            let rest = &src[at..];
            let end = rest.find(");").unwrap();
            let open = rest.find(">(").unwrap() + 2;
            rest[open..end].split(|ch: char| !(ch.is_ascii_digit() || ch == '.')).filter(|t| !t.is_empty() && *t != "3" && t.contains('.')).map(|t| t.parse::<f32>().unwrap()).collect()
        };
        let fields = body("SGN_FIELDS");
        let want: Vec<f32> = (0..c::SIGN_FIELD_COUNT).flat_map(|i| c::sign_field_tint(i).map(|v| v as f32)).collect();
        if fields != want {
            let rows: Vec<String> = want.chunks(3).map(|v| format!("vec3<f32>({:.1}, {:.1}, {:.1})", v[0], v[1], v[2])).collect();
            panic!("draw.wgsl SGN_FIELDS differs from eo-kit's sign_field_tint; it should read:\n{}", rows.join(", "));
        }
        let inks = body("SGN_INKS");
        let want: Vec<f32> = c::SIGN_INKS.iter().flat_map(|v| v.map(|x| x as f32)).collect();
        assert_eq!(inks, want, "SGN_INKS");
        assert!(src.contains(&format!("const SGN_FIELD_COUNT: u32 = {}u;", c::SIGN_FIELD_COUNT)));
        assert!(src.contains(&format!("const SGN_UV_MIN: f32 = {:?};", c::LETTER_UV_MIN)) && src.contains(&format!("const SGN_UV_SPAN: f32 = {:?};", c::LETTER_UV_SPAN)));
        assert!(src.contains(&format!("const SGN_LIT_LETTERS: u32 = {}u;", c::LETTER_LIT_LETTERS)) && src.contains(&format!("const SGN_NEON: u32 = {}u;", c::LETTER_NEON)));
        assert_eq!([c::FACE_BOLD, c::FACE_CONDENSED, c::FACE_SERIF, c::FACE_SCRIPT], [eo_props::shop_lettering::FACE_BOLD, eo_props::shop_lettering::FACE_CONDENSED, eo_props::shop_lettering::FACE_SERIF, eo_props::shop_lettering::FACE_SCRIPT]);
    }

    /// The decode draw.wgsl runs (`sgn_decode`, mirrored here on the snorm
    /// values the vertex fetch hands it) reads back every code the planner writes.
    #[test]
    fn every_code_reads_back_as_the_shader_reads_it() {
        use eo_kit::clutter::{LetterCode, SIGN_FIELD_COUNT};
        let snorm = |b: u8| (b as i8 as f32 / 127.0).max(-1.0);
        for emblem in 0..8u8 {
            for mode in 0..4u8 {
                for query in 0..3u8 {
                    for ink in 0..6u8 {
                        for field in 0..SIGN_FIELD_COUNT as u8 {
                            let code = LetterCode { cell: 200, mode, query, ink, field, emblem };
                            let b = code.bytes(0.25, 0.75);
                            let (ny, nw) = (snorm(b[3]), snorm(b[4]));
                            assert!(nw < -0.004, "a lettered vertex's w is negative");
                            let mut cc = ((((ny * 127.0).round() as i32) + 127) * 127 + (-((nw * 127.0).round() as i32) - 1)) as u32;
                            let f = cc % SIGN_FIELD_COUNT as u32;
                            cc /= SIGN_FIELD_COUNT as u32;
                            let k = cc % 6;
                            cc /= 6;
                            let q = cc % 3;
                            cc /= 3;
                            assert_eq!((cc / 4, cc % 4, q, k, f), (emblem as u32, mode as u32, query as u32, ink as u32, field as u32));
                            assert_eq!(LetterCode::decode(b).map(|d| d.0), Some(code));
                        }
                    }
                }
            }
        }
    }
}
