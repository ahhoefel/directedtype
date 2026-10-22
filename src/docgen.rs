use std::fs;
use std::path::{Path, PathBuf};

use vello::peniko::Color;

use crate::compiler::{CompiledDocument, FsResolver, ResolvedLayout};
use crate::component::ComponentRegistry;
use crate::parser::parse_document_with_file;
use crate::render::color::parse_color;
use crate::render::{HeadlessRenderer, SceneOptions};

/// Layout presentation mode for a rendered `.dt` code block in generated Markdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DocBlockLayout {
    /// Emits the ```dt code block followed by the rendered image below it.
    #[default]
    Stacked,
    /// Emits a two-column HTML `<table>` with the ```dt code block on the left and image on the right.
    SideBySide,
    /// Emits only the rendered image without the code block.
    ImageOnly,
}

/// Parsed configuration options from a ```` ```dt ... ```` code fence info string.
#[derive(Debug, Clone, PartialEq)]
pub struct DocBlockOptions {
    pub render: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub padding: u32,
    pub background: Option<Color>,
    pub scale_factor: f64,
    pub layout: DocBlockLayout,
    pub name: Option<String>,
    pub alt: Option<String>,
}

impl Default for DocBlockOptions {
    fn default() -> Self {
        Self {
            render: false,
            width: None,
            height: None,
            padding: 0,
            background: Some(Color::WHITE),
            scale_factor: 1.0,
            layout: DocBlockLayout::Stacked,
            name: None,
            alt: None,
        }
    }
}

impl DocBlockOptions {
    /// Parses space-separated attributes after `dt` on a code fence line, e.g.:
    /// `dt render width=500 height=200 padding=16 bg=#f8fafc side-by-side name=intro alt="Intro box"`
    pub fn parse_info_string(info: &str) -> Result<Option<Self>, String> {
        let trimmed = info.trim();
        let mut chars = trimmed.chars();
        // Check if it starts with "dt" followed by end-of-string or whitespace
        if !trimmed.starts_with("dt") {
            return Ok(None);
        }
        let after_dt = &trimmed[2..];
        if !after_dt.is_empty() && !after_dt.starts_with(char::is_whitespace) {
            return Ok(None);
        }
        let _ = &mut chars;

        let mut opts = DocBlockOptions::default();
        let tokens = tokenize_fence_attrs(after_dt.trim());

        for token in tokens {
            if token == "render" {
                opts.render = true;
            } else if token == "norender" || token == "ignore" {
                opts.render = false;
            } else if token == "side-by-side" || token == "side_by_side" {
                opts.layout = DocBlockLayout::SideBySide;
            } else if token == "hide-code" || token == "hide_code" || token == "image-only" || token == "image_only" {
                opts.layout = DocBlockLayout::ImageOnly;
            } else if let Some((key, val)) = token.split_once('=') {
                match key {
                    "width" | "w" => {
                        opts.width = Some(parse_px_u32(val, key)?);
                    }
                    "height" | "h" => {
                        opts.height = Some(parse_px_u32(val, key)?);
                    }
                    "padding" | "pad" => {
                        opts.padding = parse_px_u32(val, key)?;
                    }
                    "scale" => {
                        let s: f64 = val
                            .parse()
                            .map_err(|_| format!("Invalid scale value '{val}'"))?;
                        if s <= 0.0 {
                            return Err(format!("scale must be positive, got '{val}'"));
                        }
                        opts.scale_factor = s;
                    }
                    "bg" | "background" => {
                        if val.eq_ignore_ascii_case("transparent") || val.eq_ignore_ascii_case("none") {
                            opts.background = None;
                        } else {
                            opts.background = Some(parse_color(val));
                        }
                    }
                    "layout" => match val {
                        "stacked" | "below" => opts.layout = DocBlockLayout::Stacked,
                        "side-by-side" | "side_by_side" => opts.layout = DocBlockLayout::SideBySide,
                        "image-only" | "hide-code" => opts.layout = DocBlockLayout::ImageOnly,
                        other => {
                            return Err(format!(
                                "Unknown layout '{other}' (expected 'stacked', 'side-by-side', or 'hide-code')"
                            ))
                        }
                    },
                    "name" | "id" => {
                        if val.is_empty() {
                            return Err("Image name cannot be empty".to_string());
                        }
                        opts.name = Some(val.to_string());
                    }
                    "alt" => {
                        opts.alt = Some(val.to_string());
                    }
                    other => {
                        return Err(format!("Unknown dt code fence attribute '{other}={val}'"));
                    }
                }
            } else {
                return Err(format!("Unknown dt code fence attribute '{token}'"));
            }
        }

        Ok(Some(opts))
    }
}

fn parse_px_u32(val: &str, key: &str) -> Result<u32, String> {
    let stripped = val.strip_suffix("px").unwrap_or(val);
    stripped
        .parse::<u32>()
        .map_err(|_| format!("Invalid {key} value '{val}' (expected positive integer)"))
}

/// Splits fence attributes by whitespace while respecting quoted strings (e.g. `alt="Hello world"`).
fn tokenize_fence_attrs(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = '"';

    for ch in input.chars() {
        if in_quotes {
            if ch == quote_char {
                in_quotes = false;
            } else {
                current.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            in_quotes = true;
            quote_char = ch;
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Splits a raw `.dt` code block into `(compiler_source, display_source)` by processing
/// Rustdoc-style `# ` hidden lines:
/// - Lines whose trimmed start is `# ` (or whose trimmed content is `#`) have the `# ` removed
///   in `compiler_source` and are omitted from `display_source`.
/// - Lines whose trimmed start is `## ` are unescaped to `# ` in both `compiler_source` and `display_source`.
pub fn split_hidden_lines(raw_code: &str) -> (String, String) {
    let mut compiler_lines = Vec::new();
    let mut display_lines = Vec::new();

    for line in raw_code.lines() {
        let trimmed_start = line.trim_start();
        let leading_ws = &line[..line.len() - trimmed_start.len()];

        if trimmed_start == "#" {
            compiler_lines.push(String::new());
        } else if let Some(rest) = trimmed_start.strip_prefix("# ") {
            compiler_lines.push(format!("{leading_ws}{rest}"));
        } else if let Some(rest) = trimmed_start.strip_prefix("## ") {
            let unescaped = format!("{leading_ws}# {rest}");
            compiler_lines.push(unescaped.clone());
            display_lines.push(unescaped);
        } else {
            compiler_lines.push(line.to_string());
            display_lines.push(line.to_string());
        }
    }

    (compiler_lines.join("\n"), display_lines.join("\n"))
}

/// Computes the render viewport `(width, height)` and `scroll_offset` for a `ResolvedLayout`
/// according to `DocBlockOptions` (auto-sizing to content bounds when `width` or `height` is omitted).
pub fn compute_render_geometry(layout: &ResolvedLayout, opts: &DocBlockOptions) -> (u32, u32, (f64, f64)) {
    let pad = opts.padding as f64;

    if let (Some(w), Some(h)) = (opts.width, opts.height) {
        let total_w = (w + 2 * opts.padding).max(1);
        let total_h = (h + 2 * opts.padding).max(1);
        return (total_w, total_h, (-pad, -pad));
    }

    // Collect bounding box of all rendered primitives (or non-window spatial nodes if no primitives)
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    let mut found_any = false;
    for node in &layout.nodes {
        if !node.is_paint_primitive() && node.name != "PadOut" && node.name != "Box" {
            continue;
        }
        let half_border = if node.name == "Rect" {
            node.properties
                .get("border_width")
                .or_else(|| node.properties.get("stroke_width"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                .max(0.0)
                / 2.0
        } else if node.name == "Line" {
            node.properties
                .get("thickness")
                .or_else(|| node.properties.get("stroke_width"))
                .or_else(|| node.properties.get("border_width"))
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0)
                .max(0.0)
                / 2.0
        } else {
            0.0
        };
        let x0 = node.rect.x - half_border;
        let y0 = node.rect.y - half_border;
        let x1 = node.rect.x + node.rect.width + half_border;
        let y1 = node.rect.y + node.rect.height + half_border;

        if x1 > x0 || y1 > y0 {
            min_x = min_x.min(x0);
            min_y = min_y.min(y0);
            max_x = max_x.max(x1);
            max_y = max_y.max(y1);
            found_any = true;
        }
    }

    if !found_any {
        for node in &layout.nodes {
            if node.id.is_window() {
                continue;
            }
            if node.rect.width > 0.0 || node.rect.height > 0.0 {
                min_x = min_x.min(node.rect.x);
                min_y = min_y.min(node.rect.y);
                max_x = max_x.max(node.rect.x + node.rect.width);
                max_y = max_y.max(node.rect.y + node.rect.height);
                found_any = true;
            }
        }
    }

    if !found_any {
        let w = opts.width.unwrap_or(400) + 2 * opts.padding;
        let h = opts.height.unwrap_or(100) + 2 * opts.padding;
        return (w.max(1), h.max(1), (-pad, -pad));
    }

    // Determine horizontal geometry
    let (final_w, scroll_x) = if let Some(w) = opts.width {
        ((w + 2 * opts.padding).max(1), -pad)
    } else if min_x >= 0.0 && min_x <= 64.0 {
        // Content is positioned near the left origin (0..=64px): preserve origin (0,0) and add symmetric right margin `min_x`
        let auto_w = (max_x + min_x).ceil().max(1.0) as u32 + 2 * opts.padding;
        (auto_w, -pad)
    } else {
        // Content has negative coordinates or was centered far inside the default window: crop around [min_x..max_x]
        let content_w = (max_x - min_x).ceil().max(1.0) as u32 + 2 * opts.padding;
        (content_w, min_x - pad)
    };

    // Determine vertical geometry
    let (final_h, scroll_y) = if let Some(h) = opts.height {
        ((h + 2 * opts.padding).max(1), -pad)
    } else if min_y >= 0.0 && min_y <= 64.0 {
        // Content is positioned near the top origin (0..=64px): preserve origin (0,0) and add symmetric bottom margin `min_y`
        let auto_h = (max_y + min_y).ceil().max(1.0) as u32 + 2 * opts.padding;
        (auto_h, -pad)
    } else {
        // Content has negative coordinates or was centered far inside the default window: crop around [min_y..max_y]
        let content_h = (max_y - min_y).ceil().max(1.0) as u32 + 2 * opts.padding;
        (content_h, min_y - pad)
    };

    let scale = opts.scale_factor;
    let scaled_w = ((final_w as f64) * scale).ceil().max(1.0) as u32;
    let scaled_h = ((final_h as f64) * scale).ceil().max(1.0) as u32;

    (scaled_w, scaled_h, (scroll_x, scroll_y))
}

/// Output summary from compiling a `.dt.md` file.
#[derive(Debug, Clone)]
pub struct DocgenOutput {
    pub markdown: String,
    pub rendered_images: Vec<PathBuf>,
}

/// Derives the default output `.md` path for a given `.dt.md` input path.
/// For example: `docs/tutorial.dt.md` -> `docs/tutorial.md`.
pub fn default_output_md_path(input_path: &Path) -> PathBuf {
    let file_name = input_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("output.md");
    if let Some(stem) = file_name.strip_suffix(".dt.md") {
        input_path.with_file_name(format!("{stem}.md"))
    } else if file_name.ends_with(".md") {
        input_path.with_file_name(format!("{file_name}.out.md"))
    } else {
        input_path.with_extension("md")
    }
}

/// Derives the document stem used for the `images/<stem>/` subfolder.
pub fn doc_stem_from_path(input_path: &Path) -> String {
    let file_name = input_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("doc");
    if let Some(stem) = file_name.strip_suffix(".dt.md") {
        stem.to_string()
    } else {
        input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("doc")
            .to_string()
    }
}

/// Compiles a `.dt.md` file on disk into its target `.md` file and rendered PNG images.
pub fn compile_markdown_file(
    input_path: &Path,
    output_md_path: Option<&Path>,
) -> Result<DocgenOutput, String> {
    let source = fs::read_to_string(input_path)
        .map_err(|e| format!("Failed to read '{}': {e}", input_path.display()))?;

    let out_md = output_md_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| default_output_md_path(input_path));

    let out_parent = out_md.parent().unwrap_or_else(|| Path::new("."));
    let doc_stem = doc_stem_from_path(input_path);
    let image_dir = out_parent.join("images").join(&doc_stem);
    let rel_image_prefix = format!("images/{doc_stem}");

    let base_dir = input_path.parent().unwrap_or_else(|| Path::new("."));

    let mut renderer: Option<HeadlessRenderer> = None;
    let output = compile_markdown_str(
        &source,
        input_path,
        base_dir,
        &image_dir,
        &rel_image_prefix,
        &mut renderer,
    )?;

    if let Some(parent) = out_md.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create output directory '{}': {e}", parent.display()))?;
        }
    }

    fs::write(&out_md, &output.markdown)
        .map_err(|e| format!("Failed to write '{}': {e}", out_md.display()))?;

    Ok(output)
}

/// Compiles a `.dt.md` string into Markdown text and writes any rendered `.dt` code block PNGs into `image_dir`.
pub fn compile_markdown_str(
    markdown_source: &str,
    source_label: &Path,
    base_dir: &Path,
    image_dir: &Path,
    rel_image_prefix: &str,
    renderer: &mut Option<HeadlessRenderer>,
) -> Result<DocgenOutput, String> {
    let mut out_lines: Vec<String> = Vec::new();
    let mut rendered_images: Vec<PathBuf> = Vec::new();

    let lines: Vec<&str> = markdown_source.lines().collect();
    let mut i = 0;
    let mut example_counter = 0usize;

    while i < lines.len() {
        let line = lines[i];
        if let Some((fence_marker, info_str)) = parse_opening_fence(line) {
            if let Some(opts) = DocBlockOptions::parse_info_string(info_str).map_err(|e| {
                format!("{}:{}: {e}", source_label.display(), i + 1)
            })? {
                let block_start_line = i + 1;
                i += 1;
                let mut code_lines = Vec::new();
                while i < lines.len() && !is_closing_fence(lines[i], fence_marker) {
                    code_lines.push(lines[i]);
                    i += 1;
                }
                if i >= lines.len() {
                    return Err(format!(
                        "{}:{}: Unclosed ```dt code block",
                        source_label.display(),
                        block_start_line
                    ));
                }
                // Consume closing fence line
                i += 1;

                let raw_code = code_lines.join("\n");
                let (compiler_source, display_source) = split_hidden_lines(&raw_code);

                if !opts.render {
                    out_lines.push(format!("{fence_marker}dt"));
                    if !display_source.is_empty() {
                        out_lines.push(display_source);
                    }
                    out_lines.push(fence_marker.to_string());
                    continue;
                }

                example_counter += 1;
                let image_stem = opts
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("example_{example_counter}"));
                let image_filename = if image_stem.ends_with(".png") {
                    image_stem.clone()
                } else {
                    format!("{image_stem}.png")
                };

                fs::create_dir_all(image_dir).map_err(|e| {
                    format!("Failed to create image directory '{}': {e}", image_dir.display())
                })?;

                let image_path = image_dir.join(&image_filename);
                let rel_image_url = format!("{rel_image_prefix}/{image_filename}");
                let alt_text = opts
                    .alt
                    .clone()
                    .unwrap_or_else(|| format!("Example {example_counter}"));

                // Parse and compile the .dt snippet, preserving line numbers from the .dt.md file
                let padded_compiler_source =
                    format!("{}{compiler_source}", "\n".repeat(block_start_line));
                let doc =
                    parse_document_with_file(&padded_compiler_source, source_label).map_err(|e| {
                        format!(
                            "{}:{}: Parse error in ```dt render block: {e}",
                            source_label.display(),
                            block_start_line
                        )
                    })?;

                let eval_w = opts.width.unwrap_or(800) as f64;
                let eval_h = opts.height.unwrap_or(600) as f64;

                let resolver = FsResolver;
                let mut registry = ComponentRegistry::standard();
                registry.auto_discover_wasm(source_label, &resolver);
                registry.auto_discover_wasm_for_document(&doc, source_label, &resolver);

                let compiled = CompiledDocument::compile_with_registry(
                    &doc,
                    eval_w,
                    eval_h,
                    base_dir,
                    &resolver,
                    &registry,
                )
                .map_err(|e| {
                    format!(
                        "{}:{}: Compile error in ```dt render block: {e}",
                        source_label.display(),
                        block_start_line
                    )
                })?;

                let layout = compiled.layout();
                let (render_w, render_h, scroll_offset) = compute_render_geometry(&layout, &opts);

                let scene_options = SceneOptions {
                    background: opts.background,
                    scale_factor: opts.scale_factor,
                    scroll_offset,
                };

                if renderer.is_none() {
                    *renderer = Some(
                        HeadlessRenderer::new()
                            .map_err(|e| format!("Failed to initialize headless GPU renderer: {e}"))?,
                    );
                }
                let headless = renderer.as_mut().unwrap();
                headless.clear_caches();
                headless
                    .render_layout_to_file(&layout, render_w, render_h, &scene_options, &image_path)
                    .map_err(|e| {
                        format!(
                            "{}:{}: Render error in ```dt render block: {e}",
                            source_label.display(),
                            block_start_line
                        )
                    })?;

                rendered_images.push(image_path);

                match opts.layout {
                    DocBlockLayout::Stacked => {
                        out_lines.push(format!("{fence_marker}dt"));
                        if !display_source.is_empty() {
                            out_lines.push(display_source);
                        }
                        out_lines.push(fence_marker.to_string());
                        out_lines.push(String::new());
                        out_lines.push(format!("![{alt_text}]({rel_image_url})"));
                    }
                    DocBlockLayout::SideBySide => {
                        out_lines.push("<table>".to_string());
                        out_lines.push("<tr>".to_string());
                        out_lines.push("<td>".to_string());
                        out_lines.push(String::new());
                        out_lines.push(format!("{fence_marker}dt"));
                        if !display_source.is_empty() {
                            out_lines.push(display_source);
                        }
                        out_lines.push(fence_marker.to_string());
                        out_lines.push(String::new());
                        out_lines.push("</td>".to_string());
                        out_lines.push("<td>".to_string());
                        out_lines.push(String::new());
                        out_lines.push(format!("![{alt_text}]({rel_image_url})"));
                        out_lines.push(String::new());
                        out_lines.push("</td>".to_string());
                        out_lines.push("</tr>".to_string());
                        out_lines.push("</table>".to_string());
                    }
                    DocBlockLayout::ImageOnly => {
                        out_lines.push(format!("![{alt_text}]({rel_image_url})"));
                    }
                }
                continue;
            }
        }

        out_lines.push(line.to_string());
        i += 1;
    }

    let mut markdown = out_lines.join("\n");
    if markdown_source.ends_with('\n') && !markdown.ends_with('\n') {
        markdown.push('\n');
    }

    Ok(DocgenOutput {
        markdown,
        rendered_images,
    })
}

fn parse_opening_fence(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    let fence_char = trimmed.chars().next()?;
    if fence_char != '`' && fence_char != '~' {
        return None;
    }
    let fence_len = trimmed.chars().take_while(|&c| c == fence_char).count();
    if fence_len < 3 {
        return None;
    }
    let fence_marker = &trimmed[..fence_len];
    let info_str = trimmed[fence_len..].trim();
    Some((fence_marker, info_str))
}

fn is_closing_fence(line: &str, opening_marker: &str) -> bool {
    let trimmed = line.trim();
    let fence_char = opening_marker.chars().next().unwrap_or('`');
    if !trimmed.starts_with(opening_marker) {
        return false;
    }
    trimmed.chars().all(|c| c == fence_char)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_document;

    #[test]
    fn test_parse_doc_block_options() {
        assert_eq!(DocBlockOptions::parse_info_string("rust").unwrap(), None);
        assert_eq!(DocBlockOptions::parse_info_string("dt_other").unwrap(), None);

        let plain = DocBlockOptions::parse_info_string("dt").unwrap().unwrap();
        assert!(!plain.render);

        let rendered = DocBlockOptions::parse_info_string(
            "dt render width=480 height=200px padding=12 bg=#f8fafc side-by-side name=hero alt=\"Hero Card\"",
        )
        .unwrap()
        .unwrap();
        assert!(rendered.render);
        assert_eq!(rendered.width, Some(480));
        assert_eq!(rendered.height, Some(200));
        assert_eq!(rendered.padding, 12);
        assert_eq!(rendered.layout, DocBlockLayout::SideBySide);
        assert_eq!(rendered.name.as_deref(), Some("hero"));
        assert_eq!(rendered.alt.as_deref(), Some("Hero Card"));
    }

    #[test]
    fn test_split_hidden_lines() {
        let raw = "\
# \\use \"components/VStack.dt\"
# \\env font = \\Font(family: \"Arial\", size: 16px);
\\VStack(width: 200px, gap: 8px) {
    # \\let secret = 10px;
    \\Text(color: #1e293b) { \"Visible\" }
}";
        let (compiler_src, display_src) = split_hidden_lines(raw);
        assert!(compiler_src.contains("\\use \"components/VStack.dt\""));
        assert!(compiler_src.contains("\\let secret = 10px;"));
        assert!(!display_src.contains("\\use"));
        assert!(!display_src.contains("secret"));
        assert!(display_src.contains("\\Text(color: #1e293b) { \"Visible\" }"));
    }

    #[test]
    fn test_compute_render_geometry_autosize() {
        use crate::compiler::evaluate_document;

        // 1. Element with x: 20px, y: 15px, width: 100px, height: 50px -> auto-sizes with symmetric margin to 140x80
        let doc = parse_document(r#"\Rect(x: 20px, y: 15px, width: 100px, height: 50px, color: #3b82f6)"#).unwrap();
        let layout = evaluate_document(&doc).unwrap();
        let opts = DocBlockOptions::parse_info_string("dt render").unwrap().unwrap();
        let (w, h, scroll) = compute_render_geometry(&layout, &opts);
        assert_eq!((w, h), (140, 80));
        assert_eq!(scroll, (0.0, 0.0));

        // 2. Element at origin (0, 0) with padding=16 -> shifts by (-16, -16) and adds 32px to dimensions
        let doc2 = parse_document(r#"\Rect(x: 0, y: 0, width: 100px, height: 50px, color: #3b82f6)"#).unwrap();
        let layout2 = evaluate_document(&doc2).unwrap();
        let opts2 = DocBlockOptions::parse_info_string("dt render padding=16").unwrap().unwrap();
        let (w2, h2, scroll2) = compute_render_geometry(&layout2, &opts2);
        assert_eq!((w2, h2), (132, 82));
        assert_eq!(scroll2, (-16.0, -16.0));
    }
}
