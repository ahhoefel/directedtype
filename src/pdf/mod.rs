//! Pure-vector PDF backend powered by `krilla`.

pub mod links;
pub mod lower;
pub mod outline;
pub mod shadow;
pub mod text;

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use krilla::color::rgb;
use krilla::configure::{ConfigurationBuilder, PdfVersion};
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::{Document as KrillaDocument, SerializeSettings};

use crate::ast::Document as AstDocument;
use crate::compiler::compiled::CompiledDocument;
use crate::compiler::error::CompileError;
use crate::compiler::expanded::NodeId;
use crate::compiler::module::{FileResolver, FsResolver};
use crate::compiler::paginate::paginate_compiled_document;
pub use crate::compiler::paginate::{PageScene, TextSliceSpec};
use crate::component::ComponentRegistry;
use crate::dom::DomError;
use crate::error::ParseError;
use crate::parser::{parse_document, parse_document_with_file};
use crate::render::color::Rgba8;

use self::links::index_anchors;
use self::lower::render_page_scene;
use self::outline::build_document_outline;
use self::text::PdfFontCache;

/// PDF specification target version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PdfVersionOpt {
    #[default]
    Pdf17,
    Pdf20,
}

/// Configuration options controlling PDF generation and CLI fallback geometry.
#[derive(Debug, Clone)]
pub struct PdfOptions {
    pub pdf_version: PdfVersionOpt,
    pub compressed: bool,
    pub deterministic: bool,
    pub fallback_width: Option<f64>,
    pub fallback_height: Option<f64>,
    pub strict_links: bool,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            pdf_version: PdfVersionOpt::Pdf17,
            compressed: true,
            deterministic: false,
            fallback_width: None,
            fallback_height: None,
            strict_links: false,
        }
    }
}

/// Errors returned while compiling a DirectedType document to PDF.
#[derive(Debug, thiserror::Error)]
pub enum PdfExportError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Parse error: {0}")]
    Parse(#[from] ParseError),
    #[error("Compile error: {0}")]
    Compile(#[from] CompileError),
    #[error("DOM prepare error: {0}")]
    Dom(#[from] DomError),
    #[error("PDF configuration error: {0}")]
    Configuration(String),
    #[error("PDF render error: {0}")]
    Render(String),
}

#[inline]
pub(crate) fn norm_f32(val: f64) -> NormalizedF32 {
    NormalizedF32::new((val as f32).clamp(0.0, 1.0)).unwrap_or(NormalizedF32::ZERO)
}

#[inline]
pub(crate) fn rgba_to_krilla(c: Rgba8) -> (rgb::Color, NormalizedF32) {
    (
        rgb::Color::new(c.r, c.g, c.b),
        norm_f32(c.a as f64 / 255.0),
    )
}

/// Builds a 1-page `PageScene` fallback from a standard `\Window`-based `CompiledDocument`.
pub fn build_single_page_scene(compiled: &CompiledDocument, options: &PdfOptions) -> PageScene {
    let layout = Arc::new(compiled.layout.clone());
    let resolved_dpi = layout
        .get_value(NodeId::WINDOW, "dpi")
        .and_then(|v| v.as_f64())
        .unwrap_or(72.0);

    let explicit_dpi_override = compiled.expanded.window_ports.contains_key("dpi")
        && resolved_dpi > 0.0
        && (resolved_dpi - 72.0).abs() > 1e-6;
    let dpi_scale = if explicit_dpi_override {
        (72.0 / resolved_dpi) as f32
    } else {
        1.0
    };

    let mut max_w = layout
        .get_value(NodeId::WINDOW, "width")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let mut max_h = layout
        .get_value(NodeId::WINDOW, "height")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    for node in &layout.nodes {
        max_w = max_w.max(node.rect.x + node.rect.width);
        max_h = max_h.max(node.rect.y + node.rect.height);
    }

    let width = options
        .fallback_width
        .unwrap_or_else(|| max_w * dpi_scale as f64)
        .max(1.0);
    let height = options
        .fallback_height
        .unwrap_or_else(|| max_h * dpi_scale as f64)
        .max(1.0);

    PageScene {
        page_index: 0,
        page_number: 1,
        width,
        height,
        dpi_scale,
        layout,
        included_nodes: None,
        node_offsets: HashMap::new(),
        text_line_slices: HashMap::new(),
        page_node_range: None,
        flow_body_node_id: None,
    }
}

/// Lowers one or more `PageScene`s into pure-vector PDF bytes via `krilla`.
pub fn render_pages_to_pdf(
    pages: &[PageScene],
    title: Option<&str>,
    options: &PdfOptions,
) -> Result<Vec<u8>, PdfExportError> {
    let pdf_ver = match options.pdf_version {
        PdfVersionOpt::Pdf17 => PdfVersion::Pdf17,
        PdfVersionOpt::Pdf20 => PdfVersion::Pdf20,
    };
    let configuration = ConfigurationBuilder::new()
        .with_version(pdf_ver)
        .finish()
        .map_err(|e| PdfExportError::Configuration(format!("{:?}", e)))?;

    let settings = SerializeSettings {
        compress_content_streams: options.compressed,
        configuration,
        xmp_metadata: !options.deterministic,
        ..SerializeSettings::default()
    };

    let mut doc = KrillaDocument::new_with(settings);

    let mut metadata = Metadata::new().creator("DirectedType dt2pdf".to_string());
    if let Some(t) = title {
        if !t.is_empty() {
            metadata = metadata.title(t.to_string());
        }
    }
    if options.deterministic {
        metadata = metadata.document_id("directedtype-deterministic".to_string());
    }
    doc.set_metadata(metadata);

    let anchors = index_anchors(pages);
    let mut font_cache = PdfFontCache::new();

    for page_scene in pages {
        let w = page_scene.width.max(1.0) as f32;
        let h = page_scene.height.max(1.0) as f32;
        let settings = PageSettings::from_wh(w, h).ok_or_else(|| {
            PdfExportError::Render(format!("Invalid page dimensions: {}x{}pt", w, h))
        })?;
        let mut page = doc.start_page_with(settings);
        render_page_scene(
            page_scene,
            &mut page,
            &mut font_cache,
            &anchors,
            options.strict_links,
        )?;
        page.finish();
    }

    if let Some(outline) = build_document_outline(pages, &anchors) {
        doc.set_outline(outline);
    }

    doc.finish()
        .map_err(|e| PdfExportError::Render(format!("{:?}", e)))
}

/// Compiles a parsed DirectedType AST document into vector PDF bytes at `72.0` DPI.
pub fn compile_document_to_pdf<R: FileResolver>(
    doc: &AstDocument,
    base_dir: &Path,
    resolver: &R,
    registry: &ComponentRegistry,
    options: &PdfOptions,
) -> Result<Vec<u8>, PdfExportError> {
    let win_w = options.fallback_width.unwrap_or(800.0);
    let win_h = options.fallback_height.unwrap_or(600.0);
    let mut dom = crate::dom::Dom::from_document_with_window(doc, win_w, win_h)?;
    dom.prepare_with_resolver(base_dir, resolver, Some(registry))?;
    let prepared_doc = dom.to_document()?;
    let compiled = CompiledDocument::compile_with_registry_and_dpi(
        &prepared_doc,
        win_w,
        win_h,
        72.0,
        base_dir,
        resolver,
        registry,
    )?;

    let title = compiled
        .layout
        .get_value(NodeId::WINDOW, "title")
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    if !compiled.expanded.paged_flows.is_empty() {
        let paginated = paginate_compiled_document(&compiled, win_w, win_h, 72.0)?;
        return render_pages_to_pdf(&paginated.pages, title.as_deref(), options);
    }
    let scene = build_single_page_scene(&compiled, options);
    render_pages_to_pdf(&[scene], title.as_deref(), options)
}

/// Compiles an in-memory `.dt` source string into vector PDF bytes.
pub fn compile_source_to_pdf(
    source: &str,
    base_dir: &Path,
    options: &PdfOptions,
) -> Result<Vec<u8>, PdfExportError> {
    let doc = parse_document(source)?;
    let resolver = FsResolver;
    let mut registry = ComponentRegistry::standard();
    registry.auto_discover_wasm_for_document(&doc, base_dir, &resolver);
    compile_document_to_pdf(&doc, base_dir, &resolver, &registry, options)
}

/// Compiles an on-disk `.dt` file into vector PDF bytes.
pub fn compile_file_to_pdf(
    input_path: &Path,
    options: &PdfOptions,
) -> Result<Vec<u8>, PdfExportError> {
    crate::compiler::text::prewarm_font_context();
    let source = fs::read_to_string(input_path)?;
    let doc = parse_document_with_file(&source, input_path)?;
    let base_dir = input_path.parent().unwrap_or_else(|| Path::new("."));
    let resolver = FsResolver;
    let mut registry = ComponentRegistry::standard();
    registry.auto_discover_wasm(input_path, &resolver);
    registry.auto_discover_wasm_for_document(&doc, input_path, &resolver);
    compile_document_to_pdf(&doc, base_dir, &resolver, &registry, options)
}
