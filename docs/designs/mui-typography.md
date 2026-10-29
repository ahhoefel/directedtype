# Material UI Typography Design (`components/material/Typography.dt`)

## Overview
This document outlines the design breakdown for implementing Material UI's `Typography` system (`@mui/material/Typography`) in DirectedType.

---

## 1. Type Scale & Variant Presets (`TypographyVariant` + `\Font` Tokens)

MUI defines 13 standard type scale roles, each with a specific `size`, `weight`, and `line_height`:

| Variant | Font Size | Weight | Line Height | Typical Usage |
| :--- | :--- | :--- | :--- | :--- |
| `H1` | `96px` (`6rem`) | `300` | `112px` (`1.167`) | Display heading 1 |
| `H2` | `60px` (`3.75rem`) | `300` | `72px` (`1.2`) | Display heading 2 |
| `H3` | `48px` (`3rem`) | `400` | `56px` (`1.167`) | Heading 3 |
| `H4` | `34px` (`2.125rem`) | `400` | `42px` (`1.235`) | Heading 4 |
| `H5` | `24px` (`1.5rem`) | `400` | `32px` (`1.334`) | Heading 5 |
| `H6` | `20px` (`1.25rem`) | `500` | `32px` (`1.6`) | Section / Card title |
| `Subtitle1` | `16px` (`1rem`) | `400` | `28px` (`1.75`) | Regular subtitle |
| `Subtitle2` | `14px` (`0.875rem`) | `500` | `22px` (`1.57`) | Medium-weight subtitle |
| `Body1` | `16px` (`1rem`) | `400` | `24px` (`1.5`) | Default body copy |
| `Body2` | `14px` (`0.875rem`) | `400` | `20px` (`1.43`) | Secondary / dense body copy |
| `Button` | `14px` (`0.875rem`) | `500` | `24px` (`1.75`) | Button / action label |
| `Caption` | `12px` (`0.75rem`) | `400` | `20px` (`1.66`) | Helper / caption text |
| `Overline` | `12px` (`0.75rem`) | `400` | `32px` (`2.66`) | Section overline label |

### Design Options
- **Reusable `\Font` Tokens**: Export `material_font_h1` through `material_font_overline` so any component or `\env font` binding can reference the type scale directly.
- **`TypographyVariant` Enum**: `\Typography(variant: TypographyVariant = TypographyVariant.Body1)` selects the appropriate `\Font` preset.
- **Optional Shorthand Components**: Decide whether to also provide `\H1`–`\H6`, `\Subtitle1`, `\Subtitle2`, `\Body1`, `\Body2`, `\Caption`, `\Overline` wrappers around `\Typography`.

---

## 2. Content & Inline Span Ergonomics

In DirectedType, bare text inside a component's `{ ... }` content slot is synthesized into a child `\Text` node (`src/compiler/expand.rs`). If `\Typography` binds `\env font` for its selected variant and forwards properties via `\Children(...)`:
- **Plain text**: `\Typography(variant: TypographyVariant.H5) { Card Title }`
- **Rich text with inline spans / links**: `\Typography { \Text { Read the \Link(url: "...") { documentation } for details. } }`
- **Dynamic string property**: Optional `text: String` support for state-driven text.

---

## 3. Width Flow, Wrapping, and Alignment (`width` & `align`)

`Typography` naturally participates in two layout flows:
1. **Bottom-Up (Intrinsic Single-Line Width)**:
   - Used for headings, badges, or labels inside shrink-wrapping containers (`\VStack`, `\HStack`, `\Paper`).
   - `width` defaults to `only(children.width)`, pulling the measured single-line text width upward.
2. **Top-Down (Wrapping or Aligned within Container)**:
   - Used for multi-line paragraphs or aligned headings (`\Typography(width: parent.width, align: Align.Center)`).
   - Overriding `width` at the call site (Tier 4) or via a parent container's `\Children` (Tier 3) pushes `width` down to the child `\Text`, which wraps lines via Parley and computes `height` from the wrapped layout.
   - When placed inside a bottom-up `\VStack` alongside intrinsic siblings, our cycle-breaking rule automatically excludes `\Typography(width: parent.width)` from `VStack`'s `max(children.width)` calculation.

---

## 4. Text Colors & Vertical Rhythm (`color`, `text_bounds`, `gutter_bottom`)

- **Palette Text Tokens (`components/material/Palette.dt`)**:
  - `material_text_primary = #000000de` (87% black, default)
  - `material_text_secondary = #00000099` (60% black)
  - `material_text_disabled = #00000061` (38% black)
  - Support direct `color: Color = material_text_primary` (or semantic `MaterialColor` variants).
- **Vertical Metrics (`TextBounds`) & Spacing**:
  - Determine default `TextBounds` (`TextStart.Capital` → `TextEnd.Baseline` vs. full line box for multi-line text).
  - Support `gutter_bottom: Boolean = false` (`0.35em` bottom margin added to height/bottom for vertical rhythm between headings and paragraphs).

