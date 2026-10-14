use parley::layout::PositionedLayoutItem;
use parley::style::{FontFamily, FontWeight, StyleProperty};
#[cfg(not(target_os = "macos"))]
use parley::style::GenericFamily;
use parley::{Alignment, FontContext, LayoutContext};
use vello::kurbo::{Affine, BezPath, Line, Rect, RoundedRect, Stroke};
use vello::peniko::{Brush, Color, Fill};
use vello::Scene;

/// Tracks browser navigation history for forward and backward traversal.
#[derive(Debug, Clone)]
pub struct NavigationHistory {
    entries: Vec<String>,
    current_index: usize,
}

impl NavigationHistory {
    /// Creates a new navigation history initialized with a starting URL or file path.
    pub fn new(initial_url: String) -> Self {
        Self {
            entries: vec![initial_url],
            current_index: 0,
        }
    }

    /// Returns the currently active URL / path in history.
    pub fn current(&self) -> Option<&str> {
        self.entries.get(self.current_index).map(|s| s.as_str())
    }

    /// Returns true if there is a previous entry to navigate back to.
    pub fn can_go_back(&self) -> bool {
        self.current_index > 0
    }

    /// Returns true if there is a subsequent entry to navigate forward to.
    pub fn can_go_forward(&self) -> bool {
        self.current_index + 1 < self.entries.len()
    }

    /// Pushes a new URL / path onto history, discarding any forward branch.
    pub fn push(&mut self, url: String) {
        if self.current().map(|s| s == url).unwrap_or(false) {
            return;
        }
        if self.current_index + 1 < self.entries.len() {
            self.entries.truncate(self.current_index + 1);
        }
        self.entries.push(url);
        self.current_index = self.entries.len() - 1;
    }

    /// Moves one step back in history, returning the new active URL if successful.
    pub fn go_back(&mut self) -> Option<&str> {
        if self.can_go_back() {
            self.current_index -= 1;
            self.current()
        } else {
            None
        }
    }

    /// Moves one step forward in history, returning the new active URL if successful.
    pub fn go_forward(&mut self) -> Option<&str> {
        if self.can_go_forward() {
            self.current_index += 1;
            self.current()
        } else {
            None
        }
    }

    /// Returns the full list of recorded navigation history entries.
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Returns the current index within the history list.
    pub fn current_index(&self) -> usize {
        self.current_index
    }
}

/// Interactive state for the URL / address bar input field.
#[derive(Debug, Clone)]
pub struct UrlBarState {
    pub text: String,
    pub cursor_pos: usize, // Character offset (0..=text.chars().count())
    pub is_focused: bool,
    pub has_error: bool,
}

impl UrlBarState {
    /// Creates a new URL bar state with the given initial text.
    pub fn new(initial_text: String) -> Self {
        let cursor_pos = initial_text.chars().count();
        Self {
            text: initial_text,
            cursor_pos,
            is_focused: false,
            has_error: false,
        }
    }

    /// Updates the URL bar text, positioning the cursor at the end and clearing errors.
    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor_pos = self.text.chars().count();
        self.has_error = false;
    }

    /// Gives keyboard focus to the URL bar.
    pub fn focus(&mut self) {
        self.is_focused = true;
        self.cursor_pos = self.text.chars().count();
    }

    /// Removes keyboard focus from the URL bar.
    pub fn blur(&mut self) {
        self.is_focused = false;
    }

    /// Inserts a character at the current cursor position.
    pub fn insert_char(&mut self, c: char) {
        let byte_offset = self
            .text
            .char_indices()
            .nth(self.cursor_pos)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());
        self.text.insert(byte_offset, c);
        self.cursor_pos += 1;
        self.has_error = false;
    }

    /// Inserts a string at the current cursor position.
    pub fn insert_str(&mut self, s: &str) {
        let byte_offset = self
            .text
            .char_indices()
            .nth(self.cursor_pos)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());
        self.text.insert_str(byte_offset, s);
        self.cursor_pos += s.chars().count();
        self.has_error = false;
    }

    /// Deletes the character immediately preceding the cursor (Backspace).
    pub fn delete_backward(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            let byte_offset = self
                .text
                .char_indices()
                .nth(self.cursor_pos)
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.text.remove(byte_offset);
            self.has_error = false;
        }
    }

    /// Deletes the character immediately following the cursor (Delete).
    pub fn delete_forward(&mut self) {
        if self.cursor_pos < self.text.chars().count() {
            let byte_offset = self
                .text
                .char_indices()
                .nth(self.cursor_pos)
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.text.remove(byte_offset);
            self.has_error = false;
        }
    }

    /// Moves the cursor left by one character.
    pub fn move_cursor_left(&mut self) {
        self.cursor_pos = self.cursor_pos.saturating_sub(1);
    }

    /// Moves the cursor right by one character.
    pub fn move_cursor_right(&mut self) {
        self.cursor_pos = (self.cursor_pos + 1).min(self.text.chars().count());
    }

    /// Moves the cursor to the beginning of the text.
    pub fn move_cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    /// Moves the cursor to the end of the text.
    pub fn move_cursor_end(&mut self) {
        self.cursor_pos = self.text.chars().count();
    }

    /// Reverts the text to `original_url` and dismisses focus.
    pub fn cancel_editing(&mut self, original_url: &str) {
        self.set_text(original_url);
        self.blur();
    }
}

/// Identifies the interactive action triggered by clicking inside the browser navigation bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserBarHitResult {
    /// Click was outside the navigation bar or ignored.
    None,
    /// Clicked the Back button (`<`).
    Back,
    /// Clicked the Forward button (`>`).
    Forward,
    /// Clicked the Reload button (`⟳`).
    Reload,
    /// Clicked the URL bar to focus and edit.
    FocusUrlBar(Option<usize>),
    /// Clicked the DOM Inspector toggle button (`</>`).
    ToggleInspect,
    /// Click fell on non-interactive chrome within the bar.
    Consumed,
}

/// The top browser navigation bar component for the DirectedType interactive viewer.
#[derive(Debug, Clone)]
pub struct BrowserBar {
    /// Height of the navigation bar in logical pixels (default: 42.0).
    pub height: f64,
    /// Navigation history (Back / Forward).
    pub history: NavigationHistory,
    /// Interactive URL input field state.
    pub url_bar: UrlBarState,
    /// Hover state for the Back button.
    pub back_hovered: bool,
    /// Hover state for the Forward button.
    pub forward_hovered: bool,
    /// Hover state for the Reload button.
    pub reload_hovered: bool,
    /// Hover state for the URL bar.
    pub url_bar_hovered: bool,
    /// Hover state for the DOM Inspector toggle button.
    pub inspect_hovered: bool,
}

impl Default for BrowserBar {
    fn default() -> Self {
        Self::new("examples/demo.dt".to_string())
    }
}

impl BrowserBar {
    /// Creates a new `BrowserBar` with the specified initial document path or URL.
    pub fn new(initial_url: String) -> Self {
        Self {
            height: 42.0,
            history: NavigationHistory::new(initial_url.clone()),
            url_bar: UrlBarState::new(initial_url),
            back_hovered: false,
            forward_hovered: false,
            reload_hovered: false,
            url_bar_hovered: false,
            inspect_hovered: false,
        }
    }

    /// Hit-tests a mouse click at `(x, y)` given the current window width `win_w`.
    pub fn hit_test(&self, x: f64, y: f64, win_w: f64) -> BrowserBarHitResult {
        if y < 0.0 || y >= self.height {
            return BrowserBarHitResult::None;
        }

        // Back button: [8.0, 7.0, 36.0, 35.0]
        if (8.0..=36.0).contains(&x) && (7.0..=35.0).contains(&y) {
            return if self.history.can_go_back() {
                BrowserBarHitResult::Back
            } else {
                BrowserBarHitResult::Consumed
            };
        }

        // Forward button: [40.0, 7.0, 68.0, 35.0]
        if (40.0..=68.0).contains(&x) && (7.0..=35.0).contains(&y) {
            return if self.history.can_go_forward() {
                BrowserBarHitResult::Forward
            } else {
                BrowserBarHitResult::Consumed
            };
        }

        // Reload button: [72.0, 7.0, 100.0, 35.0]
        if (72.0..=100.0).contains(&x) && (7.0..=35.0).contains(&y) {
            return BrowserBarHitResult::Reload;
        }

        // Inspect button on right: [win_w - 36.0, 7.0, win_w - 8.0, 35.0]
        let inspect_x = win_w - 36.0;
        if x >= inspect_x && x <= win_w - 8.0 && (7.0..=35.0).contains(&y) {
            return BrowserBarHitResult::ToggleInspect;
        }

        // URL bar: [112.0, 6.0, inspect_x - 8.0, 36.0]
        let url_x = 112.0;
        let url_end_x = (inspect_x - 8.0).max(url_x + 60.0);
        if x >= url_x && x <= url_end_x && (6.0..=36.0).contains(&y) {
            return BrowserBarHitResult::FocusUrlBar(None);
        }

        BrowserBarHitResult::Consumed
    }

    /// Updates hover states based on current cursor position, returning `true` if any state changed.
    pub fn handle_mouse_move(&mut self, x: f64, y: f64, win_w: f64) -> bool {
        let old_back = self.back_hovered;
        let old_fwd = self.forward_hovered;
        let old_rel = self.reload_hovered;
        let old_url = self.url_bar_hovered;
        let old_ins = self.inspect_hovered;

        let inspect_x = win_w - 36.0;
        let url_x = 112.0;
        let url_end_x = (inspect_x - 8.0).max(url_x + 60.0);

        if y >= 0.0 && y < self.height {
            self.back_hovered =
                (8.0..=36.0).contains(&x) && (7.0..=35.0).contains(&y) && self.history.can_go_back();
            self.forward_hovered =
                (40.0..=68.0).contains(&x) && (7.0..=35.0).contains(&y) && self.history.can_go_forward();
            self.reload_hovered = (72.0..=100.0).contains(&x) && (7.0..=35.0).contains(&y);
            self.inspect_hovered = x >= inspect_x && x <= win_w - 8.0 && (7.0..=35.0).contains(&y);
            self.url_bar_hovered = x >= url_x && x <= url_end_x && (6.0..=36.0).contains(&y);
        } else {
            self.back_hovered = false;
            self.forward_hovered = false;
            self.reload_hovered = false;
            self.url_bar_hovered = false;
            self.inspect_hovered = false;
        }

        old_back != self.back_hovered
            || old_fwd != self.forward_hovered
            || old_rel != self.reload_hovered
            || old_url != self.url_bar_hovered
            || old_ins != self.inspect_hovered
    }

    /// Renders the complete browser navigation bar to a `vello::Scene`.
    pub fn render_to_scene(
        &self,
        scene: &mut Scene,
        win_w: f64,
        inspect_active: bool,
        font_cx: &mut FontContext,
        layout_cx: &mut LayoutContext<()>,
    ) {
        let inspect_x = win_w - 36.0;
        let url_x = 112.0;
        let url_end_x = (inspect_x - 8.0).max(url_x + 60.0);
        let _url_w = url_end_x - url_x;

        // 1. Background and bottom separator
        let bg_rect = Rect::new(0.0, 0.0, win_w, self.height);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgba8(248, 250, 252, 255)), // Slate 50
            None,
            &bg_rect,
        );

        let bottom_line = Line::new((0.0, self.height - 0.5), (win_w, self.height - 0.5));
        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgba8(226, 232, 240, 255)), // Slate 200
            None,
            &bottom_line,
        );

        // 2. Back button
        let back_rect = RoundedRect::new(8.0, 7.0, 36.0, 35.0, 6.0);
        let can_back = self.history.can_go_back();
        if can_back && self.back_hovered {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgba8(226, 232, 240, 255)),
                None,
                &back_rect,
            );
        }

        let back_color = if can_back {
            Color::from_rgba8(51, 65, 85, 255) // Slate 700
        } else {
            Color::from_rgba8(203, 213, 225, 255) // Slate 300 (disabled)
        };
        draw_back_chevron(scene, 22.0, 21.0, 11.0, back_color);

        // 3. Forward button
        let fwd_rect = RoundedRect::new(40.0, 7.0, 68.0, 35.0, 6.0);
        let can_fwd = self.history.can_go_forward();
        if can_fwd && self.forward_hovered {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgba8(226, 232, 240, 255)),
                None,
                &fwd_rect,
            );
        }

        let fwd_color = if can_fwd {
            Color::from_rgba8(51, 65, 85, 255)
        } else {
            Color::from_rgba8(203, 213, 225, 255)
        };
        draw_forward_chevron(scene, 54.0, 21.0, 11.0, fwd_color);

        // 4. Reload button
        let reload_rect = RoundedRect::new(72.0, 7.0, 100.0, 35.0, 6.0);
        if self.reload_hovered {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgba8(226, 232, 240, 255)),
                None,
                &reload_rect,
            );
        }
        let reload_color = Color::from_rgba8(71, 85, 105, 255); // Slate 600
        draw_reload_icon(scene, 86.0, 21.0, 6.0, reload_color);

        // 5. URL bar
        let url_y1 = 6.0;
        let url_y2 = 36.0;
        let url_cy = (url_y1 + url_y2) / 2.0;
        let url_rect = RoundedRect::new(url_x, url_y1, url_end_x, url_y2, 6.0);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Brush::Solid(Color::WHITE),
            None,
            &url_rect,
        );

        let (border_color, border_width) = if self.url_bar.has_error {
            (Color::from_rgba8(239, 68, 68, 255), 1.5) // Red 500
        } else if self.url_bar.is_focused {
            (Color::from_rgba8(59, 130, 246, 255), 1.5) // Blue 500
        } else if self.url_bar_hovered {
            (Color::from_rgba8(148, 163, 184, 255), 1.0) // Slate 400
        } else {
            (Color::from_rgba8(203, 213, 225, 255), 1.0) // Slate 300
        };

        scene.stroke(
            &Stroke::new(border_width),
            Affine::IDENTITY,
            Brush::Solid(border_color),
            None,
            &url_rect,
        );

        // 5a. File icon on left inside URL bar: vertically centered at url_cy with full size (10x13)
        let icon_w = 10.0;
        let icon_h = 13.0;
        let icon_fold = 3.5;
        let file_icon_color = if self.url_bar.is_focused {
            Color::from_rgba8(59, 130, 246, 255)
        } else {
            Color::from_rgba8(148, 163, 184, 255)
        };
        draw_file_icon(scene, url_x + 10.0, url_cy, icon_w, icon_h, icon_fold, file_icon_color);

        // 5b. Typographic vertical centering of URL text using tight-fitting bounding box (cap height to baseline)
        let font_size = 12.0f32;
        let cap_height = font_size as f64 * 0.71; // tight-fitting cap-height ~8.52px
        let baseline_y = url_cy + (cap_height / 2.0); // centers the [baseline - cap_height, baseline] box at url_cy

        // Text clipping boundary within URL bar
        let text_clip_rect = Rect::new(url_x + 26.0, 6.0, url_end_x - 10.0, 36.0);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &text_clip_rect);

        let text_start_x = url_x + 28.0;

        let display_text = if self.url_bar.text.is_empty() && self.url_bar.is_focused {
            "Enter path or file (e.g. examples/demo.dt)..."
        } else {
            &self.url_bar.text
        };

        let text_color = if self.url_bar.text.is_empty() && self.url_bar.is_focused {
            Color::from_rgba8(148, 163, 184, 255)
        } else {
            Color::from_rgba8(30, 41, 59, 255) // Slate 800
        };

        // Measure text before cursor to place text cursor
        let cursor_byte_idx = self
            .url_bar
            .text
            .char_indices()
            .nth(self.url_bar.cursor_pos)
            .map(|(i, _)| i)
            .unwrap_or(self.url_bar.text.len());
        let before_cursor = &self.url_bar.text[..cursor_byte_idx];
        let cursor_offset = measure_text_width(font_cx, layout_cx, before_cursor, 12.0, false);

        // Render URL bar text aligned to the exact shared baseline
        draw_text_at_baseline(
            scene,
            font_cx,
            layout_cx,
            display_text,
            12.0,
            FontWeight::NORMAL,
            text_color,
            false,
            text_start_x,
            baseline_y,
        );

        // Render blinking/solid cursor if focused
        if self.url_bar.is_focused {
            let cursor_x = text_start_x + cursor_offset;
            let cursor_line = Line::new((cursor_x, baseline_y - cap_height - 1.0), (cursor_x, baseline_y + 2.0));
            scene.stroke(
                &Stroke::new(1.5),
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgba8(37, 99, 235, 255)), // Blue 600
                None,
                &cursor_line,
            );
        }

        scene.pop_layer();

        // 6. DOM Inspector toggle button on the right
        let ins_rect = RoundedRect::new(inspect_x, 7.0, win_w - 8.0, 35.0, 6.0);
        if inspect_active {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgba8(239, 246, 255, 255)), // Blue 50
                None,
                &ins_rect,
            );
            scene.stroke(
                &Stroke::new(1.0),
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgba8(147, 197, 253, 255)), // Blue 300
                None,
                &ins_rect,
            );
        } else if self.inspect_hovered {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgba8(226, 232, 240, 255)),
                None,
                &ins_rect,
            );
        }

        let ins_icon_color = if inspect_active {
            Color::from_rgba8(37, 99, 235, 255) // Blue 600
        } else {
            Color::from_rgba8(100, 116, 139, 255) // Slate 500
        };
        draw_inspect_icon(scene, inspect_x + 14.0, 21.0, ins_icon_color);
    }
}

/// Draws a left-pointing chevron `<` for the Back button.
fn draw_back_chevron(scene: &mut Scene, cx: f64, cy: f64, size: f64, color: Color) {
    let mut path = BezPath::new();
    let half_w = size * 0.35;
    let half_h = size * 0.45;
    path.move_to((cx + half_w, cy - half_h));
    path.line_to((cx - half_w, cy));
    path.line_to((cx + half_w, cy + half_h));

    scene.stroke(
        &Stroke::new(2.0).with_caps(vello::kurbo::Cap::Round).with_join(vello::kurbo::Join::Round),
        Affine::IDENTITY,
        Brush::Solid(color),
        None,
        &path,
    );
}

/// Draws a right-pointing chevron `>` for the Forward button.
fn draw_forward_chevron(scene: &mut Scene, cx: f64, cy: f64, size: f64, color: Color) {
    let mut path = BezPath::new();
    let half_w = size * 0.35;
    let half_h = size * 0.45;
    path.move_to((cx - half_w, cy - half_h));
    path.line_to((cx + half_w, cy));
    path.line_to((cx - half_w, cy + half_h));

    scene.stroke(
        &Stroke::new(2.0).with_caps(vello::kurbo::Cap::Round).with_join(vello::kurbo::Join::Round),
        Affine::IDENTITY,
        Brush::Solid(color),
        None,
        &path,
    );
}

/// Draws a reload/refresh circular arrow icon `⟳`.
fn draw_reload_icon(scene: &mut Scene, cx: f64, cy: f64, r: f64, color: Color) {
    let mut path = BezPath::new();

    // Circular arc: start at 0.45 rad (~26° just below 3 o'clock) and sweep clockwise to 5.75 rad (~330°)
    let start_angle = 0.45f64;
    let end_angle = 1.825 * 3.1415f64;
    let steps = 24;
    for i in 0..=steps {
        let theta = start_angle + (end_angle - start_angle) * (i as f64 / steps as f64);
        let px = cx + r * theta.cos();
        let py = cy + r * theta.sin();
        if i == 0 {
            path.move_to((px, py));
        } else {
            path.line_to((px, py));
        }
    }

    // Arrowhead at arc end: straight up and straight left arms
    let tip_x = cx + r * end_angle.cos();
    let tip_y = cy + r * end_angle.sin();
    let barb_len = 5.0;

    // Arm 1: straight up
    let w1_x = tip_x;
    let w1_y = tip_y - barb_len;

    // Arm 2: straight left
    let w2_x = tip_x - barb_len;
    let w2_y = tip_y;

    // Draw the arrowhead with straight up and straight left arms
    path.move_to((w1_x, w1_y));
    path.line_to((tip_x, tip_y));
    path.line_to((w2_x, w2_y));

    scene.stroke(
        &Stroke::new(1.7)
            .with_caps(vello::kurbo::Cap::Round)
            .with_join(vello::kurbo::Join::Round),
        Affine::IDENTITY,
        Brush::Solid(color),
        None,
        &path,
    );
}

/// Draws a clean mini document file icon vertically centered at `cy`.
fn draw_file_icon(
    scene: &mut Scene,
    x: f64,
    cy: f64,
    w: f64,
    h: f64,
    fold: f64,
    color: Color,
) {
    let mut path = BezPath::new();
    let y_top = cy - h / 2.0;
    let y_bottom = cy + h / 2.0;

    path.move_to((x, y_top));
    path.line_to((x + w - fold, y_top));
    path.line_to((x + w, y_top + fold));
    path.line_to((x + w, y_bottom));
    path.line_to((x, y_bottom));
    path.close_path();

    // Corner fold line
    path.move_to((x + w - fold, y_top));
    path.line_to((x + w - fold, y_top + fold));
    path.line_to((x + w, y_top + fold));

    scene.stroke(
        &Stroke::new(1.2).with_join(vello::kurbo::Join::Round),
        Affine::IDENTITY,
        Brush::Solid(color),
        None,
        &path,
    );
}

/// Draws an inspector code/element icon `</>`.
fn draw_inspect_icon(scene: &mut Scene, cx: f64, cy: f64, color: Color) {
    let mut path = BezPath::new();
    // <
    path.move_to((cx - 3.0, cy - 4.5));
    path.line_to((cx - 7.5, cy));
    path.line_to((cx - 3.0, cy + 4.5));

    // /
    path.move_to((cx - 1.0, cy + 5.5));
    path.line_to((cx + 1.0, cy - 5.5));

    // >
    path.move_to((cx + 3.0, cy - 4.5));
    path.line_to((cx + 7.5, cy));
    path.line_to((cx + 3.0, cy + 4.5));

    scene.stroke(
        &Stroke::new(1.5).with_caps(vello::kurbo::Cap::Round).with_join(vello::kurbo::Join::Round),
        Affine::IDENTITY,
        Brush::Solid(color),
        None,
        &path,
    );
}

/// Measures single-line text width in logical pixels using Parley.
fn measure_text_width(
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    text: &str,
    font_size: f32,
    is_monospace: bool,
) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
    builder.push_default(StyleProperty::FontSize(font_size));
    if is_monospace {
        #[cfg(target_os = "macos")]
        builder.push_default(StyleProperty::FontFamily(FontFamily::named("Menlo")));
        #[cfg(not(target_os = "macos"))]
        builder.push_default(StyleProperty::FontFamily(FontFamily::Generic(GenericFamily::Monospace)));
    }
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, parley::layout::AlignmentOptions::default());
    layout.full_width() as f64
}

/// Draws single-line text using Parley and Vello aligned to an exact typographic baseline.
#[allow(clippy::too_many_arguments)]
fn draw_text_at_baseline(
    scene: &mut Scene,
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    text: &str,
    font_size: f32,
    font_weight: FontWeight,
    color: Color,
    is_monospace: bool,
    x: f64,
    baseline_y: f64,
) -> f64 {
    if text.is_empty() {
        return 0.0;
    }

    let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
    builder.push_default(StyleProperty::FontSize(font_size));
    if (font_weight.value() - 400.0).abs() > 1.0 {
        builder.push_default(StyleProperty::FontWeight(font_weight));
    }
    if is_monospace {
        #[cfg(target_os = "macos")]
        builder.push_default(StyleProperty::FontFamily(FontFamily::named("Menlo")));
        #[cfg(not(target_os = "macos"))]
        builder.push_default(StyleProperty::FontFamily(FontFamily::Generic(GenericFamily::Monospace)));
    }

    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, parley::layout::AlignmentOptions::default());

    let layout_baseline = layout
        .lines()
        .next()
        .map(|l| l.metrics().baseline as f64)
        .unwrap_or(font_size as f64 * 0.8);

    let y_offset = baseline_y - layout_baseline;
    let text_affine = Affine::translate((x, y_offset));
    for line in layout.lines() {
        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                let run = glyph_run.run();
                let font = run.font();
                let glyphs = glyph_run.positioned_glyphs().map(|g| vello::Glyph {
                    id: g.id,
                    x: g.x,
                    y: g.y,
                });

                scene
                    .draw_glyphs(font)
                    .font_size(run.font_size())
                    .transform(text_affine)
                    .brush(Brush::Solid(color))
                    .draw(Fill::NonZero, glyphs);
            }
        }
    }

    layout.full_width() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_navigation_history_initial() {
        let history = NavigationHistory::new("examples/demo.dt".to_string());
        assert_eq!(history.current(), Some("examples/demo.dt"));
        assert!(!history.can_go_back());
        assert!(!history.can_go_forward());
        assert_eq!(history.entries().len(), 1);
    }

    #[test]
    fn test_navigation_history_back_forward() {
        let mut history = NavigationHistory::new("page1.dt".to_string());
        history.push("page2.dt".to_string());
        history.push("page3.dt".to_string());

        assert_eq!(history.current(), Some("page3.dt"));
        assert!(history.can_go_back());
        assert!(!history.can_go_forward());

        assert_eq!(history.go_back(), Some("page2.dt"));
        assert!(history.can_go_back());
        assert!(history.can_go_forward());

        assert_eq!(history.go_back(), Some("page1.dt"));
        assert!(!history.can_go_back());
        assert!(history.can_go_forward());

        assert_eq!(history.go_forward(), Some("page2.dt"));
        assert_eq!(history.go_forward(), Some("page3.dt"));
        assert!(!history.can_go_forward());
    }

    #[test]
    fn test_navigation_history_branch_truncation() {
        let mut history = NavigationHistory::new("page1.dt".to_string());
        history.push("page2.dt".to_string());
        history.push("page3.dt".to_string());

        history.go_back(); // at page2
        history.push("page4.dt".to_string()); // branch: page3 discarded

        assert_eq!(history.entries(), &["page1.dt", "page2.dt", "page4.dt"]);
        assert_eq!(history.current(), Some("page4.dt"));
        assert!(!history.can_go_forward());
    }

    #[test]
    fn test_url_bar_editing() {
        let mut url_bar = UrlBarState::new("example.dt".to_string());
        assert_eq!(url_bar.cursor_pos, 10);

        url_bar.insert_char('x');
        assert_eq!(url_bar.text, "example.dtx");
        assert_eq!(url_bar.cursor_pos, 11);

        url_bar.delete_backward();
        assert_eq!(url_bar.text, "example.dt");
        assert_eq!(url_bar.cursor_pos, 10);

        url_bar.move_cursor_home();
        assert_eq!(url_bar.cursor_pos, 0);

        url_bar.insert_str("dir/");
        assert_eq!(url_bar.text, "dir/example.dt");
        assert_eq!(url_bar.cursor_pos, 4);

        url_bar.delete_forward();
        assert_eq!(url_bar.text, "dir/xample.dt");
    }

    #[test]
    fn test_browser_bar_hit_test() {
        let mut bar = BrowserBar::new("demo.dt".to_string());
        let win_w = 800.0;

        // Back is disabled initially
        assert_eq!(bar.hit_test(20.0, 20.0, win_w), BrowserBarHitResult::Consumed);

        // Enable back by navigating
        bar.history.push("second.dt".to_string());
        assert_eq!(bar.hit_test(20.0, 20.0, win_w), BrowserBarHitResult::Back);

        // Forward is disabled
        assert_eq!(bar.hit_test(50.0, 20.0, win_w), BrowserBarHitResult::Consumed);

        // Go back -> forward is enabled
        bar.history.go_back();
        assert_eq!(bar.hit_test(50.0, 20.0, win_w), BrowserBarHitResult::Forward);

        // Reload
        assert_eq!(bar.hit_test(85.0, 20.0, win_w), BrowserBarHitResult::Reload);

        // Inspect button
        assert_eq!(bar.hit_test(win_w - 20.0, 20.0, win_w), BrowserBarHitResult::ToggleInspect);

        // URL bar
        assert_eq!(bar.hit_test(200.0, 20.0, win_w), BrowserBarHitResult::FocusUrlBar(None));

        // Outside bar
        assert_eq!(bar.hit_test(200.0, 50.0, win_w), BrowserBarHitResult::None);
    }

    #[test]
    fn test_url_bar_renders_with_baseline_alignment() {
        let bar = BrowserBar::new("examples/demo.dt".to_string());
        let mut scene = Scene::new();
        let mut font_cx = FontContext::new();
        let mut layout_cx = LayoutContext::new();
        bar.render_to_scene(&mut scene, 800.0, false, &mut font_cx, &mut layout_cx);
    }
}
