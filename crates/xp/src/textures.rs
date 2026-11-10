//! Texture resolution and upload: an OBJ8 `TEXTURE` path → a bindable `wgpu::TextureView`.
//!
//! ## What the shipped content forces
//!
//! Measured over all 3 080 plain `TEXTURE` references in the shipped OBJ8 corpus, relative to the
//! directory of the referencing `.obj`:
//!
//! | outcome | references |
//! |---|---|
//! | the literal path exists | 1 056 |
//! | exists only after swapping `.png` ↔ `.dds` | 2 018 |
//! | exists only after a case-insensitive name match | 0 |
//! | nothing on disk | 6 |
//!
//! Two thirds of the install's texture references name the other extension for the file that
//! actually holds the pixels: `Aircraft/Fighters/F-4 Phantom/objects/Canopies.obj` writes
//! `TEXTURE ../objects/F4D_Fuselage.png` and the install holds
//! `Aircraft/Fighters/F-4 Phantom/objects/F4D_Fuselage.dds`. A loader that only tries the literal
//! name misses 2 018 of 3 080 references, so substitution is required by the content rather than
//! offered as a convenience.
//!
//! 167 of the 1 056 literal hits have **both** spellings on disk — for example
//! `Aircraft/General Aviation/Baron B58/objects/B58_fuselage.obj` names `B58_fuselage.png` and
//! both `B58_fuselage.png` and `B58_fuselage.dds` exist. Which one the original loads is
//! **unmeasured**: the oracle ignores `--load_acf`, so no observed run has loaded an object that
//! exercises the choice. The port takes the literal name when it exists, which is the only rule
//! that does not invent a preference, and [`Resolution::Ambiguous`] reports the cases so the
//! choice stays visible.
//!
//! Case-insensitive matching is implemented but exercised by **zero** shipped references, so it is
//! a fallback and not a load-bearing rule here. It is kept because the day/night and normal-map
//! pairing conventions measured in `docs/COMPATIBILITY.md` are case-insensitive, and a resolver
//! that behaves differently for those would be inconsistent.
//!
//! ## Upload formats
//!
//! DDS goes up as its own block-compressed format, because the install's 1 901 `.dds` files are
//! 387 BC1, 27 BC2, 1 485 BC3 and 2 uncompressed 24-bit RGB, and no file uses the DX10 extension.
//! Two of those have no matching wgpu format at all: wgpu 30 has no 24-bit texture format and no
//! single-channel 8-bit format, so 24-bit RGB DDS and every gray PNG are expanded to RGBA8 on the
//! way in.
//!
//! Textures are uploaded as **linear** (`Bc1RgbaUnorm`, `Rgba8Unorm`, …), not sRGB. That follows
//! the shaders rather than the file headers: `srgb.glsl` implements `to_linear`/`to_srgb` in GLSL
//! behind `HAS_SRGB`, and `terrain_frag.glsl:1575` does its own `pow(…, u_gamma.yyy)`, so the
//! original converts in software and samples the texture raw. The DDS header's so-called gamma bit
//! cannot drive this choice anyway: [`xp_core::tex::DdsInfo::gamma_hint`] is
//! `ddspf.dwCaps & 0x400000`, and that bit is set in all 1 901 files, so it does not discriminate.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context as _, Result};
use xp_core::png::{self, PngInfo};
use xp_core::tex::{self, DdsVariant};

/// Where a `TEXTURE` reference landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The file the object named, exactly.
    Literal(PathBuf),
    /// The named file was absent; this one has the same stem and the other extension.
    Substituted(PathBuf),
    /// The named file was absent; this one differs only in the case of its final component.
    /// No shipped reference needs this.
    CaseMatched(PathBuf),
    /// The literal name exists, and so does the other extension. The port loads the literal one;
    /// what the original loads is unmeasured.
    Ambiguous(PathBuf),
    /// Nothing on disk. The object still draws, with [`TextureStore::placeholder`].
    Missing,
}

impl Resolution {
    /// The file to read, or `None` for [`Resolution::Missing`].
    pub fn path(&self) -> Option<&Path> {
        match self {
            Resolution::Literal(p)
            | Resolution::Substituted(p)
            | Resolution::CaseMatched(p)
            | Resolution::Ambiguous(p) => Some(p),
            Resolution::Missing => None,
        }
    }
}

/// Turns the path an object wrote into a path that exists.
///
/// Directories are listed once and cached, because a scenery load asks the same directory many
/// times and the case-insensitive fallback needs the listing.
#[derive(Default)]
pub struct Resolver {
    listings: HashMap<PathBuf, Vec<String>>,
}

impl Resolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve `name` — the argument of a `TEXTURE` line, relative to `obj_dir` — to a file.
    ///
    /// An empty name is legal shipped content (82 placeholder objects write a bare `TEXTURE`), and
    /// resolves to [`Resolution::Missing`] rather than to the containing directory.
    pub fn resolve(&mut self, obj_dir: &Path, name: &str) -> Result<Resolution> {
        if name.trim().is_empty() {
            return Ok(Resolution::Missing);
        }
        // OBJ8 writes forward slashes on every platform.
        let joined = obj_dir.join(name.replace('\\', "/"));
        let literal = std::path::absolute(&joined)
            .with_context(|| format!("normalizing texture path {name:?}"))?;

        if let Some(dir) = literal.parent() {
            let entries = self.listing(dir)?;
            let stem = literal
                .file_name()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let has_other_extension = |base: &Path, taken: &str| {
                let other = if taken == ".png" { ".dds" } else { ".png" };
                let name = base
                    .file_name()
                    .map(|s| s.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if !name.ends_with(taken) {
                    return false;
                }
                let stem_only = &name[..name.len() - taken.len()];
                entries.iter().any(|e| {
                    let e = e.to_lowercase();
                    e.len() > other.len()
                        && &e[..e.len() - other.len()] == stem_only
                        && e.ends_with(other)
                })
            };
            let suffix = literal
                .extension()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            if literal.is_file() {
                return Ok(if has_other_extension(dir, &format!(".{suffix}")) {
                    Resolution::Ambiguous(literal)
                } else {
                    Resolution::Literal(literal)
                });
            }

            // Case-insensitive match on the whole final component.
            if let Some(hit) = entries.iter().find(|e| e.to_lowercase() == stem) {
                return Ok(Resolution::CaseMatched(dir.join(hit)));
            }

            // Extension substitution, then the same case-insensitive step on the substitute.
            let taken = if suffix == "png" { ".png" } else { ".dds" };
            let other = if taken == ".png" { ".dds" } else { ".png" };
            if suffix == "png" || suffix == "dds" {
                let stem_only = stem.strip_suffix(taken).ok_or_else(|| {
                    anyhow!("texture path {name:?} has no {taken} suffix to swap")
                })?;
                if let Some(hit) = entries
                    .iter()
                    .find(|e| e.to_lowercase() == format!("{stem_only}{other}"))
                {
                    return Ok(Resolution::Substituted(dir.join(hit)));
                }
            }
        }

        Ok(Resolution::Missing)
    }

    fn listing(&mut self, dir: &Path) -> Result<&Vec<String>> {
        if !self.listings.contains_key(dir) {
            let mut names = Vec::new();
            // A directory that is not there cannot contain the texture; that is a miss, not a
            // failure to load the object.
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.filter_map(|e| e.ok()) {
                    if let Some(s) = e.file_name().to_str() {
                        names.push(s.to_string());
                    }
                }
            }
            names.sort();
            self.listings.insert(dir.to_path_buf(), names);
        }
        self.listings
            .get(dir)
            .ok_or_else(|| anyhow!("no cached listing for {}", dir.display()))
    }
}

/// Decoded textures, uploaded, keyed by the file they came from.
pub struct TextureStore {
    views: HashMap<PathBuf, wgpu::TextureView>,
    /// Interface bitmaps, loaded with the original's PNG gamma table; kept apart because the
    /// same file loaded for 3-D use is not corrected here.
    ui_views: HashMap<PathBuf, wgpu::TextureView>,
    placeholder: wgpu::TextureView,
    /// The all-zero stand-in; see [`TextureStore::new`] for why a zero texel is the right answer
    /// for `tex_bump` and opaque magenta the right answer for `tex_base`.
    placeholder_zero: wgpu::TextureView,
    /// Files that decoded and uploaded.
    pub loaded: usize,
    /// References that resolved to nothing, counted per object.
    pub missing: usize,
    /// References that needed extension substitution.
    pub substituted: usize,
}

impl TextureStore {
    /// Create the store with its two 1×1 stand-ins.
    ///
    /// Two are needed because the shader treats an absent texture differently in each slot:
    ///
    /// * `tex_base` feeds `effective_a`, and `terrain_gbuffer.wgsl:375` discards a fragment whose
    ///   alpha falls to the cut level — so a transparent stand-in would make the geometry vanish
    ///   instead of showing the gap. The base placeholder is opaque magenta, which is ugly on
    ///   purpose.
    /// * `tex_bump` is checked at `:219` against an all-zero texel, which the shipped code reads
    ///   as "no normal map here" and answers with `(0.5, 0.5, 1.0, 0.0)` — a flat tangent-space
    ///   normal. Any other value becomes a real deviation: magenta would decode to
    ///   `raw = (1, -1, 1)`, and with `u_bump_scale.y = 1` the recomputed `shuffled.z` collapses
    ///   to 0, so every surface would be shaded along its own tangent axis. The bump placeholder
    ///   is therefore all zero.
    /// * `tex_comp`, `tex_comp2` and `tex_noise` are only read by the `MODE_COMPOSITE` arm, which
    ///   no OBJ8 draw selects, but WGSL still requires them bound. Zero is harmless there too.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self {
            views: HashMap::new(),
            ui_views: HashMap::new(),
            placeholder: Self::solid(
                device,
                queue,
                "missing-texture placeholder",
                [0xFF, 0x00, 0xFF, 0xFF],
            ),
            placeholder_zero: Self::solid(
                device,
                queue,
                "unbound-texture placeholder",
                [0x00, 0x00, 0x00, 0x00],
            ),
            loaded: 0,
            missing: 0,
            substituted: 0,
        }
    }

    /// The opaque stand-in for `tex_base`.
    pub fn placeholder(&self) -> &wgpu::TextureView {
        &self.placeholder
    }

    /// The all-zero stand-in for `tex_bump` and for the composite slots no OBJ8 draw reads.
    pub fn placeholder_zero(&self) -> &wgpu::TextureView {
        &self.placeholder_zero
    }

    /// A 1×1 RGBA8 texture holding one texel, usable as any binding the pipeline requires.
    fn solid(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &str,
        texel: [u8; 4],
    ) -> wgpu::TextureView {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &texel,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// The view for `resolution`, uploading it if this is the first use of that file.
    ///
    /// Returns a cloned `wgpu::TextureView` rather than a borrow so a caller can keep resolving
    /// other textures while holding one.
    pub fn get(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resolution: &Resolution,
    ) -> Result<wgpu::TextureView> {
        let path = match resolution.path() {
            Some(p) => p,
            None => {
                self.missing += 1;
                return Ok(self.placeholder.clone());
            }
        };
        if matches!(resolution, Resolution::Substituted(_)) {
            self.substituted += 1;
        }
        if let Some(view) = self.views.get(path) {
            return Ok(view.clone());
        }
        let bytes =
            std::fs::read(path).with_context(|| format!("reading texture {}", path.display()))?;
        let view = match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref()
        {
            Some("dds") => upload_dds(device, queue, path, &bytes)
                .with_context(|| format!("uploading {}", path.display()))?,
            Some("png") => upload_png(device, queue, path, &bytes)
                .with_context(|| format!("uploading {}", path.display()))?,
            other => {
                return Err(anyhow!(
                    "{}: unsupported texture extension {other:?}",
                    path.display()
                ))
            }
        };
        self.views.insert(path.to_path_buf(), view.clone());
        self.loaded += 1;
        Ok(view)
    }

    /// A PNG for the 2-D interface (panel, instruments, loading screen, atlas), run through
    /// `xp_core::png::display_lut` the way `TEX_load_png` runs it through libpng's gamma table.
    ///
    /// Whether the original sets that flag for *every* PNG, including OBJ8 textures, is not
    /// established; the 3-D path ([`TextureStore::get`]) is left uncorrected until it is.
    pub fn get_ui(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &Path,
    ) -> Result<wgpu::TextureView> {
        if let Some(view) = self.ui_views.get(path) {
            return Ok(view.clone());
        }
        let bytes =
            std::fs::read(path).with_context(|| format!("reading bitmap {}", path.display()))?;
        let lut = png::display_lut(png::gama_chunk(&bytes));
        let view = upload_png_with(device, queue, path, &bytes, Some(&lut))
            .with_context(|| format!("uploading {}", path.display()))?;
        self.ui_views.insert(path.to_path_buf(), view.clone());
        self.loaded += 1;
        Ok(view)
    }
}
/// Upload one shipped `.dds`, mip chain included.
fn upload_dds(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &Path,
    bytes: &[u8],
) -> Result<wgpu::TextureView> {
    let info = tex::parse_dds(bytes).with_context(|| format!("parsing {}", path.display()))?;
    let label = path.display().to_string();

    // wgpu 30 has no 24-bit texture format, and 2 shipped files are uncompressed 24-bit RGB, so
    // that variant has to be expanded rather than uploaded.
    if info.variant == DdsVariant::Rgb24 {
        return upload_rgb24(device, queue, path, &info, bytes);
    }

    let (format, block_bytes) = match info.variant {
        // DXT1 must map to the *RGBA* BC1 format: 41.00 % of the blocks in the shipped DXT1 files
        // use the three-colour one-bit-alpha mode, and 386 of 387 files contain at least one.
        DdsVariant::Bc1 => (wgpu::TextureFormat::Bc1RgbaUnorm, 8u32),
        DdsVariant::Bc2 => (wgpu::TextureFormat::Bc2RgbaUnorm, 16),
        DdsVariant::Bc3 => (wgpu::TextureFormat::Bc3RgbaUnorm, 16),
        DdsVariant::Rgb24 => return Err(anyhow!("{}: 24-bit RGB handled above", path.display())),
    };

    // 1 899 of the 1 901 shipped `.dds` files are BC1/2/3, and wgpu only accepts those on a device
    // created with the BC feature. Say so here: without the check the same condition surfaces as
    // a fatal validation panic inside `create_texture`.
    if !device
        .features()
        .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
    {
        return Err(anyhow!(
            "{}: {:?} needs TEXTURE_COMPRESSION_BC, which this device does not have; a CPU block \
             decoder is not ported",
            path.display(),
            info.variant
        ));
    }

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&label),
        size: wgpu::Extent3d {
            width: info.width,
            height: info.height,
            depth_or_array_layers: 1,
        },
        // The stored count, not the declared one: 357 shipped files declare N levels and hold
        // N-1, and uploading the declared count reads past the end of the payload.
        mip_level_count: info.stored_mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    for level in 0..info.stored_mips {
        let offset = (info.payload_offset + info.level_offset(level)) as usize;
        let len = info.level_bytes(level) as usize;
        let end = offset
            .checked_add(len)
            .ok_or_else(|| anyhow!("{}: level {level} offset overflows", path.display()))?;
        let data = bytes.get(offset..end).ok_or_else(|| {
            anyhow!(
                "{}: level {level} needs bytes {offset}..{end} of a {}-byte file",
                path.display(),
                bytes.len()
            )
        })?;
        let lw = (info.width >> level).max(1);
        let lh = (info.height >> level).max(1);
        // A BC row is one block wide per four texels, and the row size must be a multiple of the
        // block size.
        let bytes_per_row = lw
            .div_ceil(4)
            .checked_mul(block_bytes)
            .ok_or_else(|| anyhow!("{}: row byte count overflows", path.display()))?;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
            // The copy extent of a block-compressed level is its *physical* size: the 2×2 and 1×1
            // levels are still one whole 4×4 block, and wgpu rejects a copy that is not a multiple
            // of the block ("Copy width is not a multiple of block width").
            wgpu::Extent3d {
                width: lw.div_ceil(4) * 4,
                height: lh.div_ceil(4) * 4,
                depth_or_array_layers: 1,
            },
        );
    }

    Ok(texture.create_view(&wgpu::TextureViewDescriptor::default()))
}

/// Expand the two 24-bit RGB DDS files to RGBA8, the nearest format wgpu 30 offers.
fn upload_rgb24(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &Path,
    info: &tex::DdsInfo,
    bytes: &[u8],
) -> Result<wgpu::TextureView> {
    let label = path.display().to_string();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&label),
        size: wgpu::Extent3d {
            width: info.width,
            height: info.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: info.stored_mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for level in 0..info.stored_mips {
        let offset = (info.payload_offset + info.level_offset(level)) as usize;
        let len = info.level_bytes(level) as usize;
        let src = bytes.get(offset..offset + len).ok_or_else(|| {
            anyhow!(
                "{}: 24-bit level {level} needs {} bytes past offset {offset} of {}",
                path.display(),
                len,
                bytes.len()
            )
        })?;
        let mut rgba = Vec::with_capacity(src.len() / 3 * 4);
        for px in src.chunks_exact(3) {
            rgba.extend_from_slice(&[px[0], px[1], px[2], 0xFF]);
        }
        let lw = (info.width >> level).max(1);
        let lh = (info.height >> level).max(1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(
                    lw.checked_mul(4)
                        .ok_or_else(|| anyhow!("{}: RGBA row size overflows", path.display()))?,
                ),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: lw,
                height: lh,
                depth_or_array_layers: 1,
            },
        );
    }
    Ok(texture.create_view(&wgpu::TextureViewDescriptor::default()))
}

/// Decode one shipped `.png` and upload it as RGBA8.
fn upload_png(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &Path,
    bytes: &[u8],
) -> Result<wgpu::TextureView> {
    upload_png_with(device, queue, path, bytes, None)
}

/// [`upload_png`], with an optional table applied to the three colour channels.
fn upload_png_with(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &Path,
    bytes: &[u8],
    lut: Option<&[u8; 256]>,
) -> Result<wgpu::TextureView> {
    let info = png::decode(bytes).with_context(|| format!("decoding {}", path.display()))?;
    let mut rgba =
        to_rgba8(&info).with_context(|| format!("expanding {} to RGBA8", path.display()))?;
    if let Some(lut) = lut {
        for px in rgba.chunks_exact_mut(4) {
            px[0] = lut[px[0] as usize];
            px[1] = lut[px[1] as usize];
            px[2] = lut[px[2] as usize];
        }
    }
    let label = path.display().to_string();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&label),
        size: wgpu::Extent3d {
            width: info.width,
            height: info.height,
            depth_or_array_layers: 1,
        },
        // PNG carries no mip chain; the material sampler's mipmap filter is irrelevant at one level.
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(
                info.width
                    .checked_mul(4)
                    .ok_or_else(|| anyhow!("{}: RGBA row size overflows", path.display()))?,
            ),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: info.width,
            height: info.height,
            depth_or_array_layers: 1,
        },
    );
    Ok(texture.create_view(&wgpu::TextureViewDescriptor::default()))
}

/// Expand a decoded PNG to 8-bit RGBA.
///
/// The install's PNGs are 6 071 RGBA8, 589 RGB8, 97 gray8, 41 RGBA16, 34 interlaced RGBA8, 20
/// 1-bit indexed with tRNS, 9 gray+alpha8 and 1 interlaced RGB8. wgpu 30 has no single-channel
/// format, so gray has to expand; 16-bit is narrowed by taking the high byte of each sample, which
/// is a **port decision** — nothing in the install says how the original handled the 41 16-bit
/// files, and the oracle never loaded one.
fn to_rgba8(info: &PngInfo) -> Result<Vec<u8>> {
    let count = usize::try_from(info.width)
        .and_then(|w| usize::try_from(info.height).map(|h| w * h))
        .map_err(|_| anyhow!("{}x{} exceeds usize", info.width, info.height))?;
    let mut out = vec![0u8; count * 4];
    let s = &info.samples;

    match (info.color_type, info.bit_depth) {
        // RGBA8: the samples are already the upload layout.
        (6, 8) => {
            expect_len(s, count * 4)?;
            out.copy_from_slice(s);
        }
        // RGBA16: two bytes per channel, big-endian. Take the high byte.
        (6, 16) => {
            expect_len(s, count * 8)?;
            for (i, px) in s.chunks_exact(8).enumerate() {
                out[i * 4..i * 4 + 4].copy_from_slice(&[px[0], px[2], px[4], px[6]]);
            }
        }
        // RGB8, optionally with a tRNS colour key (six bytes: R G B, each big-endian 16-bit).
        (2, 8) => {
            expect_len(s, count * 3)?;
            let key = color_key(info)?;
            for (i, px) in s.chunks_exact(3).enumerate() {
                let a = match key {
                    Some(k)
                        if k[0] == px[0] as u16 && k[1] == px[1] as u16 && k[2] == px[2] as u16 =>
                    {
                        0
                    }
                    _ => 0xFF,
                };
                out[i * 4..i * 4 + 4].copy_from_slice(&[px[0], px[1], px[2], a]);
            }
        }
        // Gray8.
        (0, 8) => {
            expect_len(s, count)?;
            let key = color_key(info)?;
            for (i, &g) in s.iter().enumerate() {
                let a = match key {
                    Some(k) if k[0] == g as u16 => 0,
                    _ => 0xFF,
                };
                out[i * 4..i * 4 + 4].copy_from_slice(&[g, g, g, a]);
            }
        }
        // Gray+alpha8.
        (4, 8) => {
            expect_len(s, count * 2)?;
            for (i, px) in s.chunks_exact(2).enumerate() {
                out[i * 4..i * 4 + 4].copy_from_slice(&[px[0], px[0], px[0], px[1]]);
            }
        }
        // Indexed, at any of the four legal depths, with per-index alpha from tRNS.
        (3, 1) | (3, 2) | (3, 4) | (3, 8) => {
            let palette = info
                .palette
                .as_ref()
                .ok_or_else(|| anyhow!("indexed PNG with no palette"))?;
            let per_byte = 8 / info.bit_depth as usize;
            expect_len(s, count.div_ceil(per_byte))?;
            let mask = (1u16 << info.bit_depth) - 1;
            let shift = 8 - info.bit_depth as u32;
            let alphas = info.transparency.as_deref().unwrap_or(&[]);
            for i in 0..count {
                let byte = s[i / per_byte] as u16;
                let index = ((byte >> (shift * ((i % per_byte) as u32))) & mask) as usize;
                let rgb = palette.get(index).ok_or_else(|| {
                    anyhow!(
                        "pixel {i} indexes palette entry {index} of {}",
                        palette.len()
                    )
                })?;
                let a = alphas.get(index).copied().unwrap_or(0xFF);
                out[i * 4..i * 4 + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], a]);
            }
        }
        other => return Err(anyhow!("unhandled PNG colour type/bit depth {other:?}")),
    }
    Ok(out)
}

fn expect_len(s: &[u8], want: usize) -> Result<()> {
    if s.len() != want {
        return Err(anyhow!(
            "sample array is {} bytes, expected {want}",
            s.len()
        ));
    }
    Ok(())
}

/// The tRNS colour key for a non-indexed PNG, as up to three 16-bit samples.
///
/// tRNS is two bytes for gray, six for RGB, and per-index for palette; the palette form is handled
/// separately, so anything longer here is content the port does not recognise.
fn color_key(info: &PngInfo) -> Result<Option<[u16; 3]>> {
    let Some(t) = &info.transparency else {
        return Ok(None);
    };
    let channels = match info.color_type {
        0 => 1,
        2 => 3,
        _ => {
            return Err(anyhow!(
                "colour key makes no sense for type {}",
                info.color_type
            ))
        }
    };
    if t.len() != channels * 2 {
        return Err(anyhow!(
            "tRNS is {} bytes for a {channels}-channel PNG",
            t.len()
        ));
    }
    let mut key = [0xFFFFu16; 3];
    for c in 0..channels {
        key[c] = u16::from_be_bytes([t[c * 2], t[c * 2 + 1]]);
    }
    Ok(Some(key))
}
