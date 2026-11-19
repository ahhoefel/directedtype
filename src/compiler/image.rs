use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use crate::compiler::error::CompileError;
use crate::compiler::module::{normalize_path, FileResolver};
use crate::span::Span;

/// Supported image file formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Svg,
}

/// Extracted header and dimension metadata for a preloaded image asset.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageMetadata {
    pub format: ImageFormat,
    /// Unscaled pixel width (upright after EXIF orientation; for SVG, `viewBox`/pixel width).
    pub pixel_width: f64,
    /// Unscaled pixel height (upright after EXIF orientation; for SVG, `viewBox`/pixel height).
    pub pixel_height: f64,
    /// Explicit PostScript point width if an `<svg width="...">` attribute used physical units (`in`, `pt`, `mm`, `cm`, `pc`).
    pub physical_width_pts: Option<f64>,
    /// Explicit PostScript point height if an `<svg height="...">` attribute used physical units (`in`, `pt`, `mm`, `cm`, `pc`).
    pub physical_height_pts: Option<f64>,
    /// Intrinsic width-to-height ratio (`> 0.0`).
    pub aspect_ratio: f64,
    /// Embedded physical pixel density in dots per inch (PNG `pHYs` or JPEG EXIF/JFIF), if present.
    pub file_dpi: Option<f64>,
    /// EXIF orientation tag (`1..=8`, `1` = normal upright).
    pub exif_orientation: u8,
}

impl ImageMetadata {
    /// Computes the logical `Distance` width (in `px`) for Overload 1 given `dpi` and `window.dpi`.
    pub fn intrinsic_width(&self, dpi: f64, window_dpi: f64) -> f64 {
        let safe_window_dpi = if window_dpi > 0.0 { window_dpi } else { 96.0 };
        if let Some(pts) = self.physical_width_pts {
            return pts * (safe_window_dpi / 72.0);
        }
        let safe_dpi = if dpi > 0.0 { dpi } else { safe_window_dpi };
        self.pixel_width * (safe_window_dpi / safe_dpi)
    }

    /// Computes the logical `Distance` height (in `px`) for Overload 1 given `dpi` and `window.dpi`.
    pub fn intrinsic_height(&self, dpi: f64, window_dpi: f64) -> f64 {
        let safe_window_dpi = if window_dpi > 0.0 { window_dpi } else { 96.0 };
        if let Some(pts) = self.physical_height_pts {
            return pts * (safe_window_dpi / 72.0);
        }
        let safe_dpi = if dpi > 0.0 { dpi } else { safe_window_dpi };
        self.pixel_height * (safe_window_dpi / safe_dpi)
    }

    /// Returns the file's embedded DPI metadata, falling back to `window_dpi` when absent.
    pub fn resolved_file_dpi(&self, window_dpi: f64) -> f64 {
        let fallback = if window_dpi > 0.0 { window_dpi } else { 96.0 };
        self.file_dpi.filter(|&d| d > 0.0).unwrap_or(fallback)
    }
}

/// Pre-decoded image payload ready for instantaneous GPU (Vello) and PDF (Krilla) rendering.
#[derive(Clone)]
pub enum DecodedImageData {
    Raster {
        width: u32,
        height: u32,
        /// Upright RGBA8 pixel buffer (`width * height * 4` bytes, EXIF orientation already applied).
        rgba8: Arc<Vec<u8>>,
        /// Original encoded file bytes (used for zero-reencode PDF embedding when upright).
        raw_bytes: Arc<Vec<u8>>,
        /// True when `raw_bytes` is an upright (`exif_orientation == 1`) JPEG stream.
        is_raw_upright_jpeg: bool,
        /// True when `raw_bytes` is an upright PNG stream.
        is_raw_upright_png: bool,
    },
    Svg {
        /// Parsed and normalized vector `usvg::Tree`.
        tree: Arc<usvg::Tree>,
        /// Original UTF-8/XML SVG bytes.
        raw_bytes: Arc<Vec<u8>>,
    },
}

impl std::fmt::Debug for DecodedImageData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodedImageData::Raster { width, height, .. } => f
                .debug_struct("Raster")
                .field("width", width)
                .field("height", height)
                .finish(),
            DecodedImageData::Svg { tree, .. } => f
                .debug_struct("Svg")
                .field("size", &tree.size())
                .finish(),
        }
    }
}

/// A validated, preloaded, and pre-decoded image asset.
#[derive(Debug, Clone)]
pub struct LoadedImageAsset {
    pub path: String,
    pub content_hash: u64,
    pub metadata: ImageMetadata,
    pub decoded: DecodedImageData,
}

/// Document-level store of preloaded image assets, keyed by normalized resolved path string.
#[derive(Debug, Clone, Default)]
pub struct ImageAssetStore {
    assets: HashMap<String, Arc<LoadedImageAsset>>,
}

impl PartialEq for ImageAssetStore {
    fn eq(&self, other: &Self) -> bool {
        if self.assets.len() != other.assets.len() {
            return false;
        }
        self.assets.iter().all(|(k, v)| {
            other
                .assets
                .get(k)
                .is_some_and(|ov| v.content_hash == ov.content_hash)
        })
    }
}

impl Eq for ImageAssetStore {}

impl ImageAssetStore {
    pub fn new() -> Self {
        Self {
            assets: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }

    pub fn len(&self) -> usize {
        self.assets.len()
    }

    pub fn get(&self, path: &str) -> Option<&Arc<LoadedImageAsset>> {
        self.assets.get(path)
    }

    pub fn insert(&mut self, path: String, asset: Arc<LoadedImageAsset>) {
        self.assets.insert(path, asset);
    }

    pub fn merge_from(&mut self, other: &ImageAssetStore) {
        for (k, v) in &other.assets {
            self.assets.entry(k.clone()).or_insert_with(|| Arc::clone(v));
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Arc<LoadedImageAsset>)> {
        self.assets.iter()
    }

    /// Returns all filesystem paths referenced by this store (useful for file watching / hot-reload).
    pub fn referenced_paths(&self) -> Vec<PathBuf> {
        self.assets.keys().map(PathBuf::from).collect()
    }

    /// Resolves `raw_path` anchored to the authoring file in `span` (falling back to `base_dir`),
    /// reads and validates its bytes, decodes and caches the image upfront, and returns the canonical key.
    pub fn resolve_and_preload_path<R: FileResolver>(
        &mut self,
        raw_path: &str,
        span: Span,
        base_dir: &Path,
        resolver: &R,
    ) -> Result<String, CompileError> {
        if raw_path.trim().is_empty() {
            return Err(CompileError::Custom {
                message: "Image path cannot be empty".to_string(),
                span,
            });
        }

        // Fast path if already resolved to an exact key in this store
        if self.assets.contains_key(raw_path) {
            return Ok(raw_path.to_string());
        }

        let raw = Path::new(raw_path);
        let author_file_opt = span.file_path();
        let author_dir = author_file_opt
            .as_deref()
            .and_then(|p| Path::new(p).parent())
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(base_dir);

        let mut candidates: Vec<PathBuf> = Vec::new();
        if raw.is_absolute() {
            candidates.push(raw.to_path_buf());
        } else {
            candidates.push(author_dir.join(raw));
            let base_candidate = base_dir.join(raw);
            if base_candidate != candidates[0] {
                candidates.push(base_candidate);
            }
            if raw != candidates[0].as_path() {
                candidates.push(raw.to_path_buf());
            }
        }

        let mut loaded_bytes: Option<(PathBuf, Vec<u8>)> = None;
        let mut first_err: Option<String> = None;
        for cand in &candidates {
            match resolver.read_bytes(cand) {
                Ok(bytes) => {
                    loaded_bytes = Some((cand.clone(), bytes));
                    break;
                }
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
            }
        }

        let Some((resolved_path, bytes)) = loaded_bytes else {
            let anchor_desc = author_dir.display();
            return Err(CompileError::Custom {
                message: format!(
                    "Failed to load image '{}' (resolved relative to '{}'): {}",
                    raw_path,
                    anchor_desc,
                    first_err.unwrap_or_else(|| "file not found".to_string())
                ),
                span,
            });
        };

        let canonical_key = resolver
            .canonicalize(&resolved_path)
            .unwrap_or_else(|_| normalize_path(&resolved_path))
            .to_string_lossy()
            .to_string();

        if self.assets.contains_key(&canonical_key) {
            return Ok(canonical_key);
        }

        let asset = decode_and_cache_image(&canonical_key, &resolved_path, &bytes).map_err(|msg| {
            CompileError::Custom {
                message: format!("Invalid image '{}': {}", raw_path, msg),
                span,
            }
        })?;

        self.assets.insert(canonical_key.clone(), asset);
        Ok(canonical_key)
    }

    /// Preloads an image directly from filesystem path (used by host/companion runtime preloading).
    pub fn preload_fs_path(&mut self, path: &Path) -> Result<String, String> {
        let bytes = std::fs::read(path)
            .map_err(|e| format!("Failed to read image '{}': {}", path.display(), e))?;
        let canonical_key = std::fs::canonicalize(path)
            .unwrap_or_else(|_| normalize_path(path))
            .to_string_lossy()
            .to_string();
        let asset = decode_and_cache_image(&canonical_key, path, &bytes)?;
        self.assets.insert(canonical_key.clone(), asset);
        Ok(canonical_key)
    }
}

static GLOBAL_IMAGE_CACHE: OnceLock<RwLock<HashMap<(String, u64), Arc<LoadedImageAsset>>>> =
    OnceLock::new();
static GLOBAL_BY_PATH_CACHE: OnceLock<RwLock<HashMap<String, Arc<LoadedImageAsset>>>> =
    OnceLock::new();
static SVG_FONT_DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();

/// Clears cached decoded entry for a path when a file watcher detects an on-disk change.
pub fn invalidate_cached_image_path(path: &Path) {
    let key_norm = normalize_path(path).to_string_lossy().to_string();
    let key_canon = std::fs::canonicalize(path)
        .ok()
        .map(|p| p.to_string_lossy().to_string());

    if let Some(lock) = GLOBAL_IMAGE_CACHE.get() {
        if let Ok(mut map) = lock.write() {
            map.retain(|(p, _), _| {
                p != &key_norm && key_canon.as_deref() != Some(p.as_str())
            });
        }
    }
    if let Some(lock) = GLOBAL_BY_PATH_CACHE.get() {
        if let Ok(mut map) = lock.write() {
            map.remove(&key_norm);
            if let Some(c) = &key_canon {
                map.remove(c);
            }
        }
    }
}

/// Looks up a preloaded asset in the provided optional store, falling back to the global preloaded cache
/// (or on-disk fallback for companion-supplied dynamic paths).
pub fn lookup_loaded_image(
    path: &str,
    store: Option<&ImageAssetStore>,
) -> Result<Arc<LoadedImageAsset>, String> {
    if let Some(s) = store {
        if let Some(asset) = s.get(path) {
            return Ok(Arc::clone(asset));
        }
    }
    if let Some(lock) = GLOBAL_BY_PATH_CACHE.get() {
        if let Ok(map) = lock.read() {
            if let Some(asset) = map.get(path) {
                return Ok(Arc::clone(asset));
            }
        }
    }
    let fs_path = Path::new(path);
    let bytes = std::fs::read(fs_path)
        .map_err(|e| format!("Image '{}' is not preloaded and could not be read: {}", path, e))?;
    decode_and_cache_image(path, fs_path, &bytes)
}

fn compute_bytes_hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

pub fn decode_and_cache_image(
    canonical_key: &str,
    path_hint: &Path,
    bytes: &[u8],
) -> Result<Arc<LoadedImageAsset>, String> {
    let content_hash = compute_bytes_hash(bytes);
    let cache_lock = GLOBAL_IMAGE_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    if let Ok(map) = cache_lock.read() {
        if let Some(existing) = map.get(&(canonical_key.to_string(), content_hash)) {
            return Ok(Arc::clone(existing));
        }
    }

    let format = detect_image_format(bytes, path_hint)?;
    let raw_arc: Arc<Vec<u8>> = Arc::new(bytes.to_vec());

    let loaded = match format {
        ImageFormat::Png => decode_png_asset(canonical_key, content_hash, raw_arc)?,
        ImageFormat::Jpeg => decode_jpeg_asset(canonical_key, content_hash, raw_arc)?,
        ImageFormat::Svg => decode_svg_asset(canonical_key, content_hash, raw_arc)?,
    };

    let arc = Arc::new(loaded);
    if let Ok(mut map) = cache_lock.write() {
        map.insert((canonical_key.to_string(), content_hash), Arc::clone(&arc));
    }
    let by_path_lock = GLOBAL_BY_PATH_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    if let Ok(mut map) = by_path_lock.write() {
        map.insert(canonical_key.to_string(), Arc::clone(&arc));
    }
    Ok(arc)
}

fn detect_image_format(bytes: &[u8], path_hint: &Path) -> Result<ImageFormat, String> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(ImageFormat::Png);
    }
    if bytes.starts_with(b"\xFF\xD8\xFF") {
        return Ok(ImageFormat::Jpeg);
    }
    if looks_like_svg(bytes) {
        return Ok(ImageFormat::Svg);
    }

    match path_hint
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Err("File has .png extension but invalid PNG signature".to_string()),
        Some("jpg" | "jpeg") => {
            Err("File has .jpg/.jpeg extension but invalid JPEG signature".to_string())
        }
        Some("svg") => Err("File has .svg extension but does not contain valid <svg> XML".to_string()),
        Some(ext) => Err(format!(
            "Unsupported image format '.{}' (expected .png, .jpg, .jpeg, or .svg)",
            ext
        )),
        None => Err("Unrecognized image format (expected PNG, JPEG, or SVG)".to_string()),
    }
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with("<svg") {
        return true;
    }
    if (trimmed.starts_with("<?xml")
        || trimmed.starts_with("<!DOCTYPE")
        || trimmed.starts_with("<!--"))
        && trimmed.contains("<svg")
    {
        return true;
    }
    false
}

/// Snaps near-integer DPI values within PNG's 1-pixel-per-meter quantization step (`0.0254` DPI)
/// so standard densities (`72`, `96`, `144`, `150`, `192`, `300`, `600`) round cleanly.
fn snap_quantized_dpi(dpi: f64) -> f64 {
    let rounded = dpi.round();
    if rounded > 0.0 && (dpi - rounded).abs() < 0.02 {
        rounded
    } else {
        dpi
    }
}

/// Extracts physical DPI from a PNG `pHYs` chunk when `unit_specifier == 1` (meters).
pub fn parse_png_phys_dpi(bytes: &[u8]) -> Option<f64> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let mut pos = 8usize;
    while pos + 8 <= bytes.len() {
        let chunk_len = u32::from_be_bytes([
            bytes[pos],
            bytes[pos + 1],
            bytes[pos + 2],
            bytes[pos + 3],
        ]) as usize;
        let chunk_type = &bytes[pos + 4..pos + 8];
        let data_start = pos + 8;
        let Some(data_end) = data_start.checked_add(chunk_len) else {
            break;
        };
        if data_end > bytes.len() {
            break;
        }
        if chunk_type == b"pHYs" && chunk_len == 9 {
            let data = &bytes[data_start..data_end];
            let ppu_x = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
            let unit = data[8];
            if unit == 1 && ppu_x > 0 {
                return Some(snap_quantized_dpi(ppu_x as f64 * 0.0254));
            }
            return None;
        }
        if chunk_type == b"IDAT" || chunk_type == b"IEND" {
            break;
        }
        let Some(next_pos) = data_end.checked_add(4) else {
            break;
        };
        pos = next_pos;
    }
    None
}

/// Extracts `(exif_orientation, physical_dpi)` from JPEG JFIF (`APP0`) and EXIF (`APP1`) segments.
pub fn parse_jpeg_exif_and_dpi(bytes: &[u8]) -> (u8, Option<f64>) {
    if !bytes.starts_with(b"\xFF\xD8") {
        return (1, None);
    }
    let mut pos = 2usize;
    let mut orientation: u8 = 1;
    let mut jfif_dpi: Option<f64> = None;
    let mut exif_dpi: Option<f64> = None;

    while pos + 1 < bytes.len() {
        if bytes[pos] != 0xFF {
            pos += 1;
            continue;
        }
        while pos < bytes.len() && bytes[pos] == 0xFF {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let marker = bytes[pos];
        pos += 1;

        // Standalone markers without length
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            continue;
        }
        // Start of Scan or End of Image: metadata headers precede SOS
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        if pos + 2 > bytes.len() {
            break;
        }
        let seg_len = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        if seg_len < 2 || pos + seg_len > bytes.len() {
            break;
        }
        let payload = &bytes[pos + 2..pos + seg_len];

        // APP0 JFIF
        if marker == 0xE0 && payload.len() >= 12 && payload.starts_with(b"JFIF\0") {
            let units = payload[7];
            let x_density = u16::from_be_bytes([payload[8], payload[9]]);
            if x_density > 0 {
                match units {
                    1 => jfif_dpi = Some(x_density as f64),
                    2 => jfif_dpi = Some(snap_quantized_dpi(x_density as f64 * 2.54)),
                    _ => {}
                }
            }
        }
        // APP1 Exif
        else if marker == 0xE1 && payload.len() >= 14 && payload.starts_with(b"Exif\0\0") {
            let (exif_orient, parsed_exif_dpi) = parse_tiff_ifd0(&payload[6..]);
            if (1..=8).contains(&exif_orient) {
                orientation = exif_orient;
            }
            if parsed_exif_dpi.is_some() {
                exif_dpi = parsed_exif_dpi;
            }
        }

        pos += seg_len;
    }

    (orientation, exif_dpi.or(jfif_dpi))
}

fn parse_tiff_ifd0(tiff: &[u8]) -> (u8, Option<f64>) {
    if tiff.len() < 8 {
        return (1, None);
    }
    let little_endian = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return (1, None),
    };
    let read_u16 = |off: usize| -> Option<u16> {
        let b = tiff.get(off..off + 2)?;
        Some(if little_endian {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    };
    let read_u32 = |off: usize| -> Option<u32> {
        let b = tiff.get(off..off + 4)?;
        Some(if little_endian {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    };

    if read_u16(2) != Some(42) {
        return (1, None);
    }
    let Some(ifd_off_u32) = read_u32(4) else {
        return (1, None);
    };
    let ifd_off = ifd_off_u32 as usize;
    let Some(entry_count) = read_u16(ifd_off) else {
        return (1, None);
    };

    let mut orientation: u8 = 1;
    let mut x_resolution: Option<f64> = None;
    let mut res_unit: u16 = 2; // EXIF default unit is 2 (inches)

    for i in 0..(entry_count as usize) {
        let entry_off = ifd_off + 2 + i * 12;
        if entry_off + 12 > tiff.len() {
            break;
        }
        let Some(tag) = read_u16(entry_off) else {
            break;
        };
        let Some(field_type) = read_u16(entry_off + 2) else {
            break;
        };
        let Some(count) = read_u32(entry_off + 4) else {
            break;
        };

        match tag {
            // Orientation (SHORT = 3)
            0x0112 if field_type == 3 && count >= 1 => {
                if let Some(val) = read_u16(entry_off + 8) {
                    if (1..=8).contains(&val) {
                        orientation = val as u8;
                    }
                }
            }
            // XResolution (RATIONAL = 5)
            0x011A if field_type == 5 && count >= 1 => {
                if let Some(val_off) = read_u32(entry_off + 8) {
                    let roff = val_off as usize;
                    if let (Some(num), Some(den)) = (read_u32(roff), read_u32(roff + 4)) {
                        if num > 0 && den > 0 {
                            x_resolution = Some(num as f64 / den as f64);
                        }
                    }
                }
            }
            // ResolutionUnit (SHORT = 3: 1 = None, 2 = Inch, 3 = Centimeter)
            0x0128 if field_type == 3 && count >= 1 => {
                if let Some(unit_val) = read_u16(entry_off + 8) {
                    res_unit = unit_val;
                }
            }
            _ => {}
        }
    }

    let dpi = match (x_resolution, res_unit) {
        (Some(res), 2) if res > 0.0 => Some(snap_quantized_dpi(res)),
        (Some(res), 3) if res > 0.0 => Some(snap_quantized_dpi(res * 2.54)),
        _ => None,
    };

    (orientation, dpi)
}

fn apply_exif_orientation(img: image::DynamicImage, orientation: u8) -> image::DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

fn decode_png_asset(
    canonical_key: &str,
    content_hash: u64,
    raw_bytes: Arc<Vec<u8>>,
) -> Result<LoadedImageAsset, String> {
    let file_dpi = parse_png_phys_dpi(&raw_bytes);
    let dyn_img = image::load_from_memory_with_format(&raw_bytes, image::ImageFormat::Png)
        .map_err(|e| format!("PNG decode error: {}", e))?;
    let rgba = dyn_img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return Err("PNG image has zero dimensions".to_string());
    }
    let metadata = ImageMetadata {
        format: ImageFormat::Png,
        pixel_width: w as f64,
        pixel_height: h as f64,
        physical_width_pts: None,
        physical_height_pts: None,
        aspect_ratio: (w as f64) / (h as f64),
        file_dpi,
        exif_orientation: 1,
    };
    Ok(LoadedImageAsset {
        path: canonical_key.to_string(),
        content_hash,
        metadata,
        decoded: DecodedImageData::Raster {
            width: w,
            height: h,
            rgba8: Arc::new(rgba.into_raw()),
            raw_bytes,
            is_raw_upright_jpeg: false,
            is_raw_upright_png: true,
        },
    })
}

fn decode_jpeg_asset(
    canonical_key: &str,
    content_hash: u64,
    raw_bytes: Arc<Vec<u8>>,
) -> Result<LoadedImageAsset, String> {
    let (orientation, file_dpi) = parse_jpeg_exif_and_dpi(&raw_bytes);
    let dyn_img = image::load_from_memory_with_format(&raw_bytes, image::ImageFormat::Jpeg)
        .map_err(|e| format!("JPEG decode error: {}", e))?;
    let oriented = apply_exif_orientation(dyn_img, orientation);
    let rgba = oriented.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return Err("JPEG image has zero dimensions".to_string());
    }
    let metadata = ImageMetadata {
        format: ImageFormat::Jpeg,
        pixel_width: w as f64,
        pixel_height: h as f64,
        physical_width_pts: None,
        physical_height_pts: None,
        aspect_ratio: (w as f64) / (h as f64),
        file_dpi,
        exif_orientation: orientation,
    };
    Ok(LoadedImageAsset {
        path: canonical_key.to_string(),
        content_hash,
        metadata,
        decoded: DecodedImageData::Raster {
            width: w,
            height: h,
            rgba8: Arc::new(rgba.into_raw()),
            raw_bytes,
            is_raw_upright_jpeg: orientation == 1,
            is_raw_upright_png: false,
        },
    })
}

#[derive(Debug, Clone, Copy)]
enum SvgAttrDim {
    Pixels(f64),
    PhysicalPts(f64),
    Auto,
}

fn parse_svg_length_attr(raw: Option<&str>) -> Result<SvgAttrDim, String> {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(SvgAttrDim::Auto);
    };
    if s.ends_with('%') {
        return Ok(SvgAttrDim::Auto);
    }

    let unit_start = s
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '+' && c != '-' && c != 'e' && c != 'E')
        .unwrap_or(s.len());
    let (num_str, unit_str) = s.split_at(unit_start);
    let val: f64 = num_str
        .trim()
        .parse()
        .map_err(|_| format!("Invalid SVG dimension '{}'", s))?;
    if val <= 0.0 || !val.is_finite() {
        return Err(format!("SVG dimension '{}' must be positive", s));
    }

    match unit_str.trim().to_ascii_lowercase().as_str() {
        "" | "px" => Ok(SvgAttrDim::Pixels(val)),
        "pt" => Ok(SvgAttrDim::PhysicalPts(val)),
        "in" => Ok(SvgAttrDim::PhysicalPts(val * 72.0)),
        "pc" => Ok(SvgAttrDim::PhysicalPts(val * 12.0)),
        "mm" => Ok(SvgAttrDim::PhysicalPts(val * (72.0 / 25.4))),
        "cm" => Ok(SvgAttrDim::PhysicalPts(val * (72.0 / 2.54))),
        // Fallback for em/ex/other CSS units: let usvg resolve at 16px/em
        _ => Ok(SvgAttrDim::Auto),
    }
}

fn parse_svg_viewbox(raw: Option<&str>) -> Option<(f64, f64)> {
    let s = raw?.trim();
    let nums: Vec<f64> = s
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|tok| !tok.is_empty())
        .filter_map(|tok| tok.parse::<f64>().ok())
        .collect();
    if nums.len() == 4 && nums[2] > 0.0 && nums[3] > 0.0 {
        Some((nums[2], nums[3]))
    } else {
        None
    }
}

fn decode_svg_asset(
    canonical_key: &str,
    content_hash: u64,
    raw_bytes: Arc<Vec<u8>>,
) -> Result<LoadedImageAsset, String> {
    let xml_str =
        std::str::from_utf8(&raw_bytes).map_err(|e| format!("SVG is not valid UTF-8: {}", e))?;

    let xml_doc = roxmltree::Document::parse(xml_str)
        .map_err(|e| format!("SVG XML parse error: {}", e))?;
    let svg_root = xml_doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "svg")
        .ok_or_else(|| "No <svg> root element found".to_string())?;

    let w_attr = parse_svg_length_attr(svg_root.attribute("width"))?;
    let h_attr = parse_svg_length_attr(svg_root.attribute("height"))?;
    let vb_dims = parse_svg_viewbox(svg_root.attribute("viewBox"));

    let has_text_elem = xml_doc
        .descendants()
        .any(|n| n.is_element() && n.tag_name().name() == "text");

    let mut opt = usvg::Options::default();
    if let Some(parent) = Path::new(canonical_key).parent() {
        opt.resources_dir = Some(parent.to_path_buf());
    }
    if has_text_elem {
        let db = SVG_FONT_DB.get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        });
        opt.fontdb = Arc::clone(db);
    }

    let tree = usvg::Tree::from_str(xml_str, &opt)
        .map_err(|e| format!("SVG parse error: {}", e))?;

    // Validate that either explicit width/height or a valid viewBox is present
    if matches!(w_attr, SvgAttrDim::Auto)
        && matches!(h_attr, SvgAttrDim::Auto)
        && vb_dims.is_none()
    {
        return Err(
            "SVG image must specify either positive width/height attributes or a valid viewBox"
                .to_string(),
        );
    }

    let tree_w = tree.size().width() as f64;
    let tree_h = tree.size().height() as f64;
    let vb_ratio = vb_dims.map(|(vw, vh)| vw / vh);

    let (pixel_width, physical_width_pts, pixel_height, physical_height_pts) = match (w_attr, h_attr)
    {
        (SvgAttrDim::Pixels(w), SvgAttrDim::Pixels(h)) => (w, None, h, None),
        (SvgAttrDim::PhysicalPts(w_pt), SvgAttrDim::PhysicalPts(h_pt)) => {
            (w_pt * (96.0 / 72.0), Some(w_pt), h_pt * (96.0 / 72.0), Some(h_pt))
        }
        (SvgAttrDim::Pixels(w), SvgAttrDim::PhysicalPts(h_pt)) => {
            (w, None, h_pt * (96.0 / 72.0), Some(h_pt))
        }
        (SvgAttrDim::PhysicalPts(w_pt), SvgAttrDim::Pixels(h)) => {
            (w_pt * (96.0 / 72.0), Some(w_pt), h, None)
        }
        (SvgAttrDim::Pixels(w), SvgAttrDim::Auto) => {
            let h = vb_ratio.map(|r| w / r).unwrap_or(tree_h);
            (w, None, h, None)
        }
        (SvgAttrDim::Auto, SvgAttrDim::Pixels(h)) => {
            let w = vb_ratio.map(|r| h * r).unwrap_or(tree_w);
            (w, None, h, None)
        }
        (SvgAttrDim::PhysicalPts(w_pt), SvgAttrDim::Auto) => {
            let h_pt = vb_ratio
                .map(|r| w_pt / r)
                .unwrap_or(tree_h * (72.0 / 96.0));
            (
                w_pt * (96.0 / 72.0),
                Some(w_pt),
                h_pt * (96.0 / 72.0),
                Some(h_pt),
            )
        }
        (SvgAttrDim::Auto, SvgAttrDim::PhysicalPts(h_pt)) => {
            let w_pt = vb_ratio
                .map(|r| h_pt * r)
                .unwrap_or(tree_w * (72.0 / 96.0));
            (
                w_pt * (96.0 / 72.0),
                Some(w_pt),
                h_pt * (96.0 / 72.0),
                Some(h_pt),
            )
        }
        (SvgAttrDim::Auto, SvgAttrDim::Auto) => {
            let (vw, vh) = vb_dims.unwrap_or((tree_w, tree_h));
            (vw, None, vh, None)
        }
    };

    // Aspect ratio in consistent units (using 96 DPI reference for mixed physical/px)
    let norm_w = physical_width_pts
        .map(|pt| pt * (96.0 / 72.0))
        .unwrap_or(pixel_width);
    let norm_h = physical_height_pts
        .map(|pt| pt * (96.0 / 72.0))
        .unwrap_or(pixel_height);
    if norm_w <= 0.0 || norm_h <= 0.0 {
        return Err("SVG image resolved to non-positive dimensions".to_string());
    }

    let metadata = ImageMetadata {
        format: ImageFormat::Svg,
        pixel_width,
        pixel_height,
        physical_width_pts,
        physical_height_pts,
        aspect_ratio: norm_w / norm_h,
        file_dpi: None,
        exif_orientation: 1,
    };

    Ok(LoadedImageAsset {
        path: canonical_key.to_string(),
        content_hash,
        metadata,
        decoded: DecodedImageData::Svg {
            tree: Arc::new(tree),
            raw_bytes,
        },
    })
}
