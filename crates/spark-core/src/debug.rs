//! Debug overlay and diagnostics for Spark games.
//!
//! Shows FPS, frame time, object count, draw calls, GPU info, etc.
//! on top of the game. The runner toggles it with F1 unless [`DebugOverlay::hotkey`] is off.

use std::collections::VecDeque;

use crate::color::Color;

/// Number of frame times kept for the average.
const HISTORY: usize = 120;

/// One line of debug text.
#[derive(Clone, Debug)]
pub struct DebugLine {
    pub text: String,
    pub color: Color,
}

/// Debug overlay state. Added to `World` and drawn by the runner.
#[derive(Clone, Debug)]
pub struct DebugOverlay {
    enabled: bool,
    show_fps: bool,
    show_gpu: bool,
    show_scene: bool,
    show_physics: bool,
    lines: Vec<DebugLine>,
    /// Last [`HISTORY`] frame times in ms.
    frame_times: VecDeque<f32>,
    /// Whether the runner toggles the overlay with F1 (games that use F1 can turn this off).
    pub hotkey: bool,
    pub fps: f32,
    pub frame_time_ms: f32,
    pub min_frame_ms: f32,
    pub max_frame_ms: f32,
}

impl Default for DebugOverlay {
    fn default() -> Self {
        Self {
            enabled: false,
            show_fps: true,
            show_gpu: true,
            show_scene: true,
            show_physics: false,
            lines: Vec::new(),
            frame_times: VecDeque::with_capacity(HISTORY),
            hotkey: true,
            fps: 0.0,
            frame_time_ms: 0.0,
            min_frame_ms: f32::MAX,
            max_frame_ms: 0.0,
        }
    }
}

impl DebugOverlay {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    pub fn show_fps(&self) -> bool {
        self.show_fps
    }
    pub fn set_show_fps(&mut self, on: bool) {
        self.show_fps = on;
    }
    pub fn show_gpu(&self) -> bool {
        self.show_gpu
    }
    pub fn set_show_gpu(&mut self, on: bool) {
        self.show_gpu = on;
    }
    pub fn show_scene(&self) -> bool {
        self.show_scene
    }
    pub fn set_show_scene(&mut self, on: bool) {
        self.show_scene = on;
    }
    pub fn show_physics(&self) -> bool {
        self.show_physics
    }
    pub fn set_show_physics(&mut self, on: bool) {
        self.show_physics = on;
    }

    pub fn add_line(&mut self, text: impl Into<String>, color: Color) {
        self.lines.push(DebugLine {
            text: text.into(),
            color,
        });
    }

    /// Called every frame with the real frame time.
    pub fn record_frame(&mut self, dt: f32) {
        let ms = dt * 1000.0;
        self.frame_time_ms = ms;
        if self.frame_times.len() == HISTORY {
            self.frame_times.pop_front();
        }
        self.frame_times.push_back(ms);
        self.min_frame_ms = self.min_frame_ms.min(ms);
        self.max_frame_ms = self.max_frame_ms.max(ms);
        if ms > 0.0 {
            self.fps = 1000.0 / ms;
        }
    }

    /// Average frame time in ms over the last 120 frames.
    pub fn avg_frame_ms(&self) -> f32 {
        if self.frame_times.is_empty() {
            0.0
        } else {
            self.frame_times.iter().sum::<f32>() / self.frame_times.len() as f32
        }
    }

    /// FPS averaged over the last 120 frames (steadier than [`DebugOverlay::fps`]).
    pub fn avg_fps(&self) -> f32 {
        let avg = self.avg_frame_ms();
        if avg > 0.0 { 1000.0 / avg } else { 0.0 }
    }

    pub fn reset_min_max(&mut self) {
        self.min_frame_ms = f32::MAX;
        self.max_frame_ms = 0.0;
    }

    /// Clear user-added lines. The runner calls this every frame (also while the overlay is hidden)
    /// so lines added each frame never pile up.
    pub fn clear_lines(&mut self) {
        self.lines.clear();
    }

    /// Returns the debug lines to draw this frame.
    pub fn build_lines(
        &self,
        gpu_name: &str,
        object_count: usize,
        draw_calls: u32,
        triangles: u64,
        physics_active: usize,
    ) -> Vec<DebugLine> {
        let mut lines = Vec::new();
        let white = Color::WHITE;
        let yellow = Color::rgb(1.0, 1.0, 0.0);
        let green = Color::rgb(0.5, 1.0, 0.5);
        let cyan = Color::rgb(0.5, 1.0, 1.0);

        if self.show_fps {
            let fps = self.avg_fps();
            // Before the first frame min is still f32::MAX: show 0 instead of a 39-digit number.
            let min = if self.frame_times.is_empty() { 0.0 } else { self.min_frame_ms };
            lines.push(DebugLine {
                text: format!(
                    "FPS: {fps:.0} | frame: {:.2} ms (min {min:.1} / max {:.1} / avg {:.1})",
                    self.frame_time_ms,
                    self.max_frame_ms,
                    self.avg_frame_ms()
                ),
                color: if fps >= 55.0 {
                    green
                } else if fps >= 30.0 {
                    yellow
                } else {
                    Color::rgb(1.0, 0.3, 0.3)
                },
            });
        }
        if self.show_gpu {
            lines.push(DebugLine {
                text: format!("GPU: {}", gpu_name),
                color: cyan,
            });
        }
        if self.show_scene {
            lines.push(DebugLine {
                text: format!(
                    "Objects: {} | Draw calls: {} | Triangles: {}",
                    object_count, draw_calls, triangles
                ),
                color: white,
            });
        }
        if self.show_physics && physics_active > 0 {
            lines.push(DebugLine {
                text: format!("Physics bodies: {}", physics_active),
                color: white,
            });
        }
        // User-added lines
        lines.extend(self.lines.iter().cloned());
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;

    #[test]
    fn toggle_on_off() {
        let mut d = DebugOverlay::new();
        assert!(!d.enabled());
        d.toggle();
        assert!(d.enabled());
        d.toggle();
        assert!(!d.enabled());
    }

    #[test]
    fn record_frame_updates_stats() {
        let mut d = DebugOverlay::new();
        d.record_frame(0.016);
        assert!((d.frame_time_ms - 16.0).abs() < 0.01);
        assert!((d.fps - 62.5).abs() < 0.01);
    }

    #[test]
    fn min_max_tracking() {
        let mut d = DebugOverlay::new();
        d.record_frame(0.010); // 10ms
        d.record_frame(0.020); // 20ms
        d.record_frame(0.015); // 15ms
        assert!((d.min_frame_ms - 10.0).abs() < 0.1);
        assert!((d.max_frame_ms - 20.0).abs() < 0.1);
    }

    #[test]
    fn avg_frame_time() {
        let mut d = DebugOverlay::new();
        for _ in 0..10 {
            d.record_frame(0.016);
        }
        assert!((d.avg_frame_ms() - 16.0).abs() < 0.01);
        assert!((d.avg_fps() - 62.5).abs() < 0.01);
    }

    #[test]
    fn history_is_bounded() {
        let mut d = DebugOverlay::new();
        for _ in 0..HISTORY {
            d.record_frame(0.010);
        }
        for _ in 0..HISTORY {
            d.record_frame(0.020);
        }
        assert_eq!(d.frame_times.len(), HISTORY);
        assert!((d.avg_frame_ms() - 20.0).abs() < 0.01);
    }

    #[test]
    fn no_garbage_before_first_frame() {
        let d = DebugOverlay::new();
        let lines = d.build_lines("gpu", 0, 0, 0, 0);
        assert!(lines[0].text.contains("min 0.0"), "{}", lines[0].text);
    }

    #[test]
    fn add_custom_line() {
        let mut d = DebugOverlay::new();
        d.add_line("custom", Color::WHITE);
        let lines = d.build_lines("Test GPU", 10, 5, 100, 0);
        assert!(lines.iter().any(|l| l.text == "custom"));
    }

    #[test]
    fn reset_min_max() {
        let mut d = DebugOverlay::new();
        d.record_frame(0.020);
        d.reset_min_max();
        assert_eq!(d.min_frame_ms, f32::MAX);
        assert_eq!(d.max_frame_ms, 0.0);
    }
}
