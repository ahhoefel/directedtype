/// Reusable DTML component definition for the translucent bounding box highlight.
pub const INSPECT_HIGHLIGHT_DTML: &str = r#"
\Component InspectHighlight(
    x: Number: 0,
    y: Number: 0,
    width: Number: 100,
    height: Number: 100,
    fill_color: Color: #38bdf820,
    border_color: Color: #38bdf8,
    border_width: Number: 1.5,
    z: Number: 9990
) {
    \Rect(
        x: x,
        y: y,
        width: width,
        height: height,
        color: fill_color,
        border_color: border_color,
        border_width: border_width,
        z: z
    )
}
"#;

/// Reusable DTML component definition for the floating coordinate/tag badge.
pub const INSPECT_BADGE_DTML: &str = r#"
\Component InspectBadge(
    target_x: Number: 0,
    target_y: Number: 0,
    target_w: Number: 100,
    target_h: Number: 50,
    label: String: "",
    bg_color: Color: #0f172a,
    text_color: Color: #38bdf8,
    z: Number: 9999
) {
    \Rect(
        x: max(target_x, 4),
        y: target_y >= 26 ? target_y - 24 : target_y + target_h + 4,
        width: 160,
        height: 20,
        radius: 4,
        color: bg_color,
        z: z
    ) {
        \Text(
            x: parent.x + 8,
            y: parent.y + 3,
            color: text_color,
            font_size: 11
        ) { label }
    }
}
"#;

/// Reusable DTML component definition for visual clip context bounds.
pub const INSPECT_CLIP_GUIDE_DTML: &str = r#"
\Component InspectClipGuide(
    x: Number: 0,
    y: Number: 0,
    width: Number: 100,
    height: Number: 100,
    stroke_color: Color: #a855f7,
    z: Number: 9980
) {
    \Rect(
        x: x,
        y: y,
        width: width,
        height: height,
        color: transparent,
        border_color: stroke_color,
        border_width: 1,
        z: z
    )
}
"#;

/// Composite DTML component combining highlight, badge, and clip guides.
pub const INSPECT_OVERLAY_DTML: &str = r#"
\Component InspectOverlay(
    target_x: Number,
    target_y: Number,
    target_w: Number,
    target_h: Number,
    label: String: "",
    is_selected: Bool: false,
    has_clip: Bool: false,
    clip_x: Number: 0,
    clip_y: Number: 0,
    clip_w: Number: 0,
    clip_h: Number: 0
) {
    \InspectHighlight(
        x: target_x,
        y: target_y,
        width: target_w,
        height: target_h,
        fill_color: is_selected ? #f59e0b28 : #38bdf820,
        border_color: is_selected ? #f59e0b : #38bdf8,
        border_width: is_selected ? 2.0 : 1.5
    )
    \InspectBadge(
        target_x: target_x,
        target_y: target_y,
        target_w: target_w,
        target_h: target_h,
        label: label,
        text_color: is_selected ? #f59e0b : #38bdf8
    )
}
"#;

/// Returns all standard inspector DTML component definitions combined into a single snippet.
pub fn standard_inspector_components() -> String {
    format!(
        "{}\n{}\n{}\n{}",
        INSPECT_HIGHLIGHT_DTML.trim(),
        INSPECT_BADGE_DTML.trim(),
        INSPECT_CLIP_GUIDE_DTML.trim(),
        INSPECT_OVERLAY_DTML.trim(),
    )
}
