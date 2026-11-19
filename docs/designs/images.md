# DirectedType Design Document: Image Assets (`Path`, `\Image`, & `\InlineImage`)

- **Status:** Active
- **Date:** November 2026
- **Target Area:** Type System & AST (`src/ast.rs`, `src/compiler/value.rs`, `src/compiler/expand/typecheck.rs`), Asset Loader & Preloader (`src/compiler/image.rs`), Built-in Primitive (`src/compiler/expand/builtins/image.rs`), Standard Library (`components/InlineImage.dt`), Vello Renderer (`src/render/scene.rs`), PDF Backend (`src/pdf/lower.rs`)

---

## 1. Executive Summary & Goals

DirectedType supports embedding raster (**PNG**, **JPEG**) and vector (**SVG**) images referenced by file path across both the interactive GPU renderer (**Vello**) and the vector print backend (**Krilla PDF**).

The design adheres to four core invariants of DirectedType:

1. **Source-Anchored `Path` Type Safety:** Image sources have static type `Path` rather than `String`. String literals (`"logo.svg"`) coerce to `Path` anchored to **the directory of the `.dt` file where the string literal is written** (via `Span::file_path()`). Arbitrary `String` variables or string concatenations do *not* coerce to `Path`, preventing components from accidentally re-anchoring caller paths against the component definition's directory.
2. **Upfront Validation & Preloading by Default:** Because `Path` values in `.dt` cannot be synthesized by runtime string concatenation, every `Path` reachable from `.dt` markup—including inactive branches of ternaries such as `is_dark ? "dark.svg" : "light.svg"`—is statically discoverable at compile time. All referenced images are validated during compilation and preloaded/decoded upfront into an asset store so reactive state toggles swap assets instantaneously with zero lazy-loading flicker.
3. **Aspect-Preserving Disjoint Overloads (No `auto`, No Distortion):** `\Image` exposes three mutually disjoint constructor overloads:
   - Intrinsic / DPI-sized (`dpi: Float = window.dpi`, where `1` raster pixel defaults to `1px × 1px`, or uses embedded PNG/JPEG DPI via `dpi: img.file_dpi`),
   - Width-constrained (`width: Distance` $\rightarrow$ `height = width / aspect_ratio`),
   - Height-constrained (`height: Distance` $\rightarrow$ `width = height * aspect_ratio`).
   Specifying both `width` and `height` is omitted by design so aspect ratios are never distorted; cropping uses standard `\Clip` composition.
4. **Clean Separation of Block (`\Image`) and Inline (`\InlineImage`):** `\Image` is the minimal spatial paint primitive (requiring explicit `x` and `y` without defaults, matching `\Rect` and `\Text`). `\InlineImage` is a user-space `\InlineBox` component in `components/InlineImage.dt` that wraps `\Image`, requires `x` and `y` to be wired by Parley (or an explicit container), and adds `baseline_offset: Distance = height`.

---

## 2. The `Path` Type & Relative Path Resolution

### 2.1 Motivation

Suppose `components/BrandHeader.dt` defines a reusable header component and `examples/app.dt` imports it:

```dt
// components/BrandHeader.dt
\Component BrandHeader(avatar: Path, x: Distance, y: Distance) {
    \Image(src: "brand_mark.svg", x: x, y: y, height: 24px)
    \Image(src: avatar, x: x + 120px, y: y, height: 24px)
}

// examples/app.dt
\use "components/BrandHeader.dt"
\BrandHeader(avatar: "alice.jpg", x: 16px, y: 16px)
```

- `"brand_mark.svg"` was written in `components/BrandHeader.dt` $\rightarrow$ resolves relative to `components/`.
- `"alice.jpg"` was written in `examples/app.dt` $\rightarrow$ resolves relative to `examples/`.
- If `BrandHeader` mistakenly declared `avatar: String` and passed `avatar` to `\Image(src: avatar, ...)`, compilation fails with `TypeMismatch: expected Path, found String`.

### 2.2 Coercion & Resolution Rules

- **Type Representation:**
  - `StaticType::Path` in `src/compiler/expand/typecheck.rs` (`canonical_name() == "Path"`).
  - `Literal::Path(String, Span)` in `src/ast.rs` and `Value::Path(String)` in `src/compiler/value.rs`, holding the normalized resolved path.
- **Literal Coercion:**
  - A `Literal::String(raw, span)` coerces to `Literal::Path(resolved, span)` whenever checked against `StaticType::Path`:
    - Direct component or primitive ports (`\Image(src: "foo.png", ...)`),
    - Typed bindings (`\let p: Path = "foo.png";`, `\env logo: Path = "logo.svg";`, `\state icon: Path = "idle.svg";`),
    - Component parameter defaults (`\Component Icon(src: Path = "default.svg", ...)`),
    - Conditional branches (`is_dark ? "dark.svg" : "light.svg"` and `match` arms) whose expected type is `Path`.
- **Anchor Directory Resolution Order:**
  Given `Literal::String(raw, span)`:
  1. If `raw` is an absolute filesystem path, normalize `raw`.
  2. Otherwise, determine `anchor_dir` from `span.file_path()`'s parent directory (falling back to compilation `base_dir` when compiling an unnamed in-memory snippet).
  3. Check `anchor_dir.join(raw)` via `FileResolver::read_bytes`. If found, normalize and use it.
  4. Otherwise, fall back to `base_dir.join(raw)` and workspace-relative `raw` (matching `\use` resolution in `src/compiler/module.rs`).
- **Supported Operators on `Path`:**
  - Equality and inequality (`==`, `!=` $\rightarrow$ `Boolean`), conditional selection (`?:`, `match`), and component/state passing.
  - No string concatenation (`+`) or implicit conversion between `String` and `Path`.

### 2.3 Upfront Asset Manifest & Preloading

Every `Literal::Path` resolved during document expansion is immediately loaded through `FileResolver::read_bytes`, validated, decoded, and stored in an `ImageAssetStore` attached to `ExpandedDocument` and `CompiledDocument`:

- **Compile-Time Diagnostics:** Missing files, unsupported image formats, corrupt headers, or zero-sized dimensions fail compilation immediately at the offending string literal's `Span`.
- **Zero-Flicker Reactive Swaps:** All assets in the document manifest (e.g. both `"dark.svg"` and `"light.svg"` in `is_dark ? "dark.svg" : "light.svg"`) are pre-decoded at compile/load time, making reactive state transitions instantaneous.
- **Virtual Filesystem Compatibility:** Retaining `Arc<ImageAssetStore>` on `CompiledDocument` allows `VirtualResolver`-based tests and embedded hosts to compile and render in-memory PNG, JPEG, and SVG bytes without touching `std::fs`.
- **Companion Rust / WASM Support:** `directedtype-guest` and the host `Context` expose a typed `Path` wrapper and `preload_image` hook so companion code mutating `\state` with runtime-selected files can preload assets explicitly before swapping state.

---

## 3. Image Formats, Intrinsic Dimensions, & DPI Metadata

### 3.1 Supported Formats

Format detection inspects magic bytes first (with extension fallback):

| Format | Magic / Signature | Dimensions & Orientation | Embedded Physical Density (`file_dpi`) |
| :--- | :--- | :--- | :--- |
| **PNG** | `\x89PNG\r\n\x1a\n` | `IHDR` `width × height` (pixels) | Optional `pHYs` chunk when `unit == 1` (meters): `dpi = pixels_per_meter * 0.0254`. Absent or `unit == 0` $\rightarrow$ `None`. |
| **JPEG** | `\xFF\xD8\xFF` | `SOF` frame `width × height` (pixels); swapped when EXIF Orientation $\in \{5, 6, 7, 8\}$ | Optional EXIF `APP1` (`XResolution` + `ResolutionUnit` inch/`cm`) or JFIF `APP0` (`density_units` `1`=inch, `2`=cm). Aspect-only (`0`) or absent $\rightarrow$ `None`. |
| **SVG** | XML `<svg ...>` | Root `width` / `height` attributes, falling back to `viewBox="minX minY w h"`. Missing/zero both $\rightarrow$ compile error. | `None` (physical units on `<svg width="2in">` are lowered directly via `window.dpi / 72.0`). |

### 3.2 Logical Pixels, Retina Scaling, & `dpi`

In DirectedType:
- `1in` equals `window.dpi` logical pixels (`96.0` in the Viewer, `72.0` in PDF export).
- On a `2.0` scale-factor Retina screen, each `1px` logical unit spans `2 × 2` physical hardware pixels.

For raster images (and unitless/`px` SVGs):
$$\text{scale} = \frac{\text{window.dpi}}{\text{dpi}}$$
$$\text{intrinsic\_width} = \text{pixel\_width} \times \frac{\text{window.dpi}}{\text{dpi}} \times 1\text{px}, \qquad \text{intrinsic\_height} = \text{pixel\_height} \times \frac{\text{window.dpi}}{\text{dpi}} \times 1\text{px}$$

- **Default (`dpi = window.dpi`):** $\text{scale} = 1.0$, so `1` raster pixel maps to `1px × 1px` (`2 × 2` physical pixels on a `2x` Retina display, matching CSS/web).
- **Explicit or File-Supplied DPI (`dpi: 300` or `dpi: img.file_dpi`):** Scales the logical box to its exact physical size (`pixel_width / dpi` inches) in both Viewer (`96` DPI) and PDF (`72` DPI) while retaining full source resolution when painting.
- **Missing Metadata Fallback:** When a file has no embedded physical DPI tag, `file_dpi` evaluates to `window.dpi` (producing `scale = 1.0`).
- **Physical-Unit SVGs:** If `<svg>` specifies physical units (`pt`, `in`, `pc`, `mm`, `cm`) on `width`/`height`, those dimensions convert to PostScript points and lower via `pts * (window.dpi / 72.0) * 1px`.

---

## 4. `\Image` Built-in Primitive Specification

`\Image` is a leaf paint primitive (`is_paint_primitive() == true`). Passing children in a content block (`\Image(...) { ... }`) is a compile-time error.

### 4.1 Three Mutually Disjoint Sizing Overloads

`src`, `x`, and `y` are required on all overloads (never defaulted):

1. **Overload 1 — Intrinsic / DPI-Sized:**
   ```dt
   \Image(
       src: Path,
       x: Distance,
       y: Distance,
       dpi: Float = window.dpi,
       ...visual_ports
   )
   ```
   - Required ports: `{src, x, y}`
   - Synthesized aliases:
     - `width: Distance = __image_intrinsic_width(src, dpi, window.dpi)`
     - `height: Distance = __image_intrinsic_height(src, dpi, window.dpi)`

2. **Overload 2 — Width-Constrained:**
   ```dt
   \Image(
       src: Path,
       x: Distance,
       y: Distance,
       width: Distance,
       ...visual_ports
   )
   ```
   - Required ports: `{src, x, y, width}` (`height` and `dpi` are rejected)
   - Synthesized alias:
     - `height: Distance = width / aspect_ratio`

3. **Overload 3 — Height-Constrained:**
   ```dt
   \Image(
       src: Path,
       x: Distance,
       y: Distance,
       height: Distance,
       ...visual_ports
   )
   ```
   - Required ports: `{src, x, y, height}` (`width` and `dpi` are rejected)
   - Synthesized alias:
     - `width: Distance = height * aspect_ratio`

Because `width ∉ All(O1)`, `height ∉ All(O1)`, `height ∉ All(O2)`, and `width ∉ All(O3)`, no two overloads satisfy $\text{Req}(O_i) \subseteq \text{All}(O_j) \land \text{Req}(O_j) \subseteq \text{All}(O_i)$.

### 4.2 Read-Only Aliases & Optional Ports

- **Read-only aliases (cannot be overridden by callers):**
  - `left`, `top`, `right`, `bottom` (`Distance`)
  - `aspect_ratio` (`Float`, equal to `intrinsic_width / intrinsic_height`)
  - `file_dpi` (`Float`, embedded file DPI or `window.dpi` if absent)
  - `width` / `height` (whichever are not explicit inputs in the active overload)
- **Optional visual, accessibility, and flow ports:**
  - Corner radii (`Distance`): `radius`, `radius_left`, `radius_right`, `radius_top`, `radius_bottom`, `radius_top_left`, `radius_top_right`, `radius_bottom_right`, `radius_bottom_left`
  - Border & shadow: `border_width: Distance`, `border_color: Color`, `shadow: Shadow`
  - Presentation & metadata: `opacity: Float` (`1.0`), `alt: String` (`""`), `z: Integer` (`0`)
  - Flow, clipping, & interaction: `clip: Clip`, `pagination: Pagination`, `cursor: String`, `name: String`, `path: String`, `id: String`, `on_*: Handler`

### 4.3 Usage Examples

```dt
// 1. Default CSS 1px-per-pixel sizing inside a VStack (which wires x and y):
\VStack(x: 24px, y: 24px, gap: 12px) {
    \Image(src: "diagrams/architecture.svg", width: 480px)
    \Image(src: "photos/team.jpg", width: 480px, radius: 12px)
}

// 2. Honoring embedded print/Retina DPI metadata via self-referencing let:
\let fig = \Image(
    src: "figures/plot_300dpi.png",
    x: 32px,
    y: 32px,
    dpi: fig.file_dpi
)

// 3. Instantaneous preloaded dark/light swap:
\Image(
    src: is_dark ? "icons/moon.svg" : "icons/sun.svg",
    x: 16px,
    y: 16px,
    height: 20px
)
```

---

## 5. `\InlineImage` Standard Component (`components/InlineImage.dt`)

`\InlineImage` is defined in `components/InlineImage.dt` using `\InlineBox` with three matching disjoint overloads (`dpi`, `width`, or `height`):

```dt
// 1. Intrinsic / DPI-sized inline image
\InlineBox InlineImage(
    src: Path,
    x: Distance,
    y: Distance,
    env font: Font = \Font(size: 16px),
    dpi: Float = window.dpi,
    baseline_offset: Distance = height,
    radius: Distance = 0px,
    border_width: Distance = 0px,
    border_color: Color = #00000000,
    opacity: Float = 1.0,
    alt: String = ""
) {
    \let img = \Image(src, x, y, dpi, radius, border_width, border_color, opacity, alt)
    \alias width: Distance = img.width;
    \alias height: Distance = img.height;
    \alias aspect_ratio: Float = img.aspect_ratio;
    \alias file_dpi: Float = img.file_dpi;
    \alias baseline: Distance = y + baseline_offset;
}
```
*(Plus Overload 2 requiring `width: Distance` and Overload 3 requiring `height: Distance`.)*

- **Required `x` and `y` Contract:** `x` and `y` have no defaults. When `\InlineImage` is placed inside `\Text` or `\Span`, the compiler's inline-box overload selector and Parley `span_metric` pass supply `x` and `y` automatically. Placing `\InlineImage` outside text flow without a container or caller providing `x` and `y` is a compile-time error.
- **Baseline Alignment:** `baseline_offset` measures the vertical distance from the image's `top` (`y`) down to the text line's baseline. Its default `height` places the bottom edge of the image flush on the baseline; passing `baseline_offset: (height + font.cap_height) / 2` centers an icon vertically against uppercase letters.

---

## 6. Rendering & Pagination Pipeline

### 6.1 Vello GPU Backend (`src/render/scene.rs`)

- **Raster (PNG / JPEG):** Pre-decoded upright RGBA8 pixels are wrapped into a cached `vello::peniko::Image` (`Blob<u8>`) in an `ImageSceneCache` and painted into `(node.rect.x, node.rect.y, node.rect.width, node.rect.height)` via `Affine::translate((x, y)) * Affine::scale_non_uniform(sx, sy)`.
- **Vector (SVG):** Parsed via `usvg::Tree` and lowered into a cached local `vello::Scene` at unscaled SVG `viewBox`/size coordinates, then appended with an `Affine` transform into the target box—preserving infinite vector sharpness at any DPI or zoom factor.
- **Shadows, Rounding, Borders, & Opacity:**
  - Drop shadow (`shadow_*`) is drawn behind the image box (matching `\Rect`).
  - When any corner radius $> 0$ (or `opacity < 1.0`), Vello pushes a rounded-rect clip/alpha layer before drawing the image, then strokes `border_width` / `border_color` on top.

### 6.2 Krilla PDF Backend (`src/pdf/lower.rs`)

- **JPEG:** When EXIF orientation is normal (`1`), raw JPEG bytes embed directly via `krilla::image::Image::from_jpeg` without re-encoding or inflating PDF size; rotated EXIF orientations (`2..=8`) embed losslessly from the oriented pixel buffer.
- **PNG:** Raw PNG bytes embed losslessly via `krilla::image::Image::from_png` (with alpha channel preserved as PDF SMask).
- **SVG:** Lowered directly from the parsed `usvg::Tree` into native `krilla::surface::Surface` vector paths, fills, strokes, linear/radial gradients, clip paths, and embedded sub-images—keeping PDF output crisp and selectable/zoomable as true vector graphics.
- **Rounding, Shadows, & Borders:** Uses `build_rect_path` clip paths, `draw_rect_shadow`, and border strokes identically to `\Rect`.

### 6.3 Pagination (`src/compiler/paginate.rs`) & Viewer Hot-Reload

- **Pagination:** `\Image` is treated as an atomic, unsplittable spatial box inside `\PagedFlow` (if `y + height` exceeds the remaining column height and the image is not at the top of an empty page, it advances cleanly to the next page).
- **Live Viewer:** Referenced asset paths in `CompiledDocument` are registered with the file watcher in `src/bin/viewer.rs` so editing an `.svg`, `.png`, or `.jpg` on disk hot-reloads the scene automatically.
