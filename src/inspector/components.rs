/// Reusable DTML component definition for the translucent bounding box highlight.
pub const INSPECT_HIGHLIGHT_DTML: &str = r#"
\Component InspectHighlight(
    x: Distance = 0,
    y: Distance = 0,
    width: Distance = 100px,
    height: Distance = 100px,
    fill_color: Color = #38bdf820,
    border_color: Color = #38bdf8,
    border_width: Distance = 1.5px,
    z: Integer = 9990
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
    target_x: Distance = 0,
    target_y: Distance = 0,
    target_w: Distance = 100px,
    target_h: Distance = 50px,
    label: String = "",
    bg_color: Color = #0f172a,
    text_color: Color = #38bdf8,
    z: Integer = 9999
) {
    \Rect(
        x: max(target_x, 4px),
        y: target_y >= 26px ? target_y - 24px : target_y + target_h + 4px,
        width: 160px,
        height: 20px,
        radius: 4px,
        color: bg_color,
        z: z
    ) {
        \Text(
            x: parent.x + 8px,
            y: parent.y + 3px,
            color: text_color,
            font: \Font(size: 11px)
        ) { label }
    }
}
"#;

/// Reusable DTML component definition for visual clip context bounds.
pub const INSPECT_CLIP_GUIDE_DTML: &str = r#"
\Component InspectClipGuide(
    x: Distance = 0,
    y: Distance = 0,
    width: Distance = 100px,
    height: Distance = 100px,
    stroke_color: Color = #a855f7,
    z: Integer = 9980
) {
    \Rect(
        x: x,
        y: y,
        width: width,
        height: height,
        color: transparent,
        border_color: stroke_color,
        border_width: 1px,
        z: z
    )
}
"#;

/// Composite DTML component combining highlight, badge, and clip guides.
pub const INSPECT_OVERLAY_DTML: &str = r#"
\Component InspectOverlay(
    target_x: Distance,
    target_y: Distance,
    target_w: Distance,
    target_h: Distance,
    label: String = "",
    is_selected: Boolean = false,
    has_clip: Boolean = false,
    clip_x: Distance = 0,
    clip_y: Distance = 0,
    clip_w: Distance = 0,
    clip_h: Distance = 0
) {
    \InspectHighlight(
        x: target_x,
        y: target_y,
        width: target_w,
        height: target_h,
        fill_color: is_selected ? #f59e0b28 : #38bdf820,
        border_color: is_selected ? #f59e0b : #38bdf8,
        border_width: is_selected ? 2.0px : 1.5px
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
