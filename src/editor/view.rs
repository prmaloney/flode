//! The canvas view: how a screen point maps to the editor and back.

use nih_plug_iced::{mouse, Point, Rectangle, Vector};

/// The closest and farthest the canvas zooms.
pub(super) const ZOOM_MIN: f32 = 0.25;
pub(super) const ZOOM_MAX: f32 = 3.0;

/// The canvas view: nodes are drawn at `zoom` and moved by `offset`. A screen point is
/// `world * zoom + offset`. The pill and the mouse position stay in screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct View {
    pub(super) zoom: f32,
    pub(super) offset: Vector,
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            offset: Vector::new(0.0, 0.0),
        }
    }
}

impl View {
    /// The editor point under a screen point.
    pub(super) fn to_world(&self, screen: Point) -> Point {
        Point::new(
            (screen.x - self.offset.x) / self.zoom,
            (screen.y - self.offset.y) / self.zoom,
        )
    }

    /// The screen point an editor point is drawn at.
    #[cfg(test)]
    pub(super) fn to_screen(&self, world: Point) -> Point {
        Point::new(
            world.x * self.zoom + self.offset.x,
            world.y * self.zoom + self.offset.y,
        )
    }

    /// The editor rectangle that covers a screen rectangle.
    pub(super) fn rect_to_world(&self, rect: Rectangle) -> Rectangle {
        let origin = self.to_world(Point::new(rect.x, rect.y));
        Rectangle {
            x: origin.x,
            y: origin.y,
            width: rect.width / self.zoom,
            height: rect.height / self.zoom,
        }
    }

    /// Moves the content by a screen offset.
    pub(super) fn pan(&mut self, by: Vector) {
        self.offset = Vector::new(self.offset.x + by.x, self.offset.y + by.y);
    }

    /// Scales by `factor` around a screen point. The editor point under that point stays put.
    pub(super) fn zoom_at(&mut self, screen: Point, factor: f32) {
        let world = self.to_world(screen);
        self.zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        self.offset = Vector::new(
            screen.x - world.x * self.zoom,
            screen.y - world.y * self.zoom,
        );
    }
}

/// A pixel count for one line of a mouse wheel. Trackpads already report pixels.
pub(super) const LINE_SCROLL_PIXELS: f32 = 24.0;

/// What a wheel or trackpad event does to the view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum ViewScroll {
    /// Scale by `1 + magnification`.
    Zoom(f32),
    /// Move the content by this many screen pixels.
    Pan(Vector),
}

/// Reads a scroll event as a zoom or a pan. The baseview fork sends a pinch as a line delta with
/// a NaN x, because iced has no pinch event. Any other scroll pans.
pub(super) fn view_scroll(delta: mouse::ScrollDelta) -> ViewScroll {
    match delta {
        mouse::ScrollDelta::Lines { x, y } if x.is_nan() => ViewScroll::Zoom(y),
        mouse::ScrollDelta::Lines { x, y } => {
            ViewScroll::Pan(Vector::new(x * LINE_SCROLL_PIXELS, y * LINE_SCROLL_PIXELS))
        }
        mouse::ScrollDelta::Pixels { x, y } => ViewScroll::Pan(Vector::new(x, y)),
    }
}
