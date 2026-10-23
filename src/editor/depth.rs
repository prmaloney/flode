//! The cues that give the flat design some depth: soft shadows under raised things, a dot grid
//! behind the graph, and a glow around wires. The canvas has no blur, so each one is a few stacked
//! translucent shapes. Where the shapes overlap they add up, which gives a soft falloff.

use super::draw::rounded_rect;
use nih_plug_iced::widget::canvas::{Frame, Path};
use nih_plug_iced::{Color, Point, Rectangle, Size};

/// How a shadow looks: how far it reaches past the edge, how far it drops below the shape, and
/// how dark the shape's own edge is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Shadow {
    pub reach: f32,
    pub drop: f32,
    pub strength: f32,
}

/// A node at rest.
pub(super) const NODE_SHADOW: Shadow = Shadow {
    reach: 9.0,
    drop: 3.0,
    strength: 0.26,
};
/// The selected node, lifted a little further off the canvas.
pub(super) const LIFTED_SHADOW: Shadow = Shadow {
    reach: 16.0,
    drop: 6.0,
    strength: 0.34,
};
/// Menus, lists, and the burger button, which float over the nodes.
pub(super) const FLOAT_SHADOW: Shadow = Shadow {
    reach: 11.0,
    drop: 4.0,
    strength: 0.34,
};

/// How many shapes make up a shadow.
const SHADOW_LAYERS: usize = 6;

/// Draws a soft shadow under a rounded rectangle. Call it before filling the shape itself.
pub(super) fn draw_shadow(frame: &mut Frame, bounds: Rectangle, radius: f32, shadow: Shadow) {
    let alpha = shadow.strength / SHADOW_LAYERS as f32;
    // The widest layer goes first. Each narrower layer darkens the middle a little more.
    for layer in (1..=SHADOW_LAYERS).rev() {
        let spread = shadow.reach * layer as f32 / SHADOW_LAYERS as f32;
        let rect = Rectangle {
            x: bounds.x - spread,
            y: bounds.y - spread + shadow.drop,
            width: bounds.width + 2.0 * spread,
            height: bounds.height + 2.0 * spread,
        };
        frame.fill(
            &rounded_rect(rect, radius + spread),
            Color::from_rgba(0.0, 0.0, 0.0, alpha),
        );
    }
}

/// The dots in the grid are this many editor units apart. Every fourth one is a major dot.
pub(super) const GRID_STEP: f32 = 24.0;
const GRID_MAJOR_EVERY: i64 = 4;
/// Below this zoom the minor dots are gone, and the major dots alone are left. Fully drawn from
/// the second value up.
const MINOR_FADE: (f32, f32) = (0.6, 0.85);
/// Below this zoom the whole grid is gone, so a view zoomed far out stays plain.
const GRID_FADE: (f32, f32) = (0.2, 0.35);
const MINOR_ALPHA: f32 = 0.05;
const MAJOR_ALPHA: f32 = 0.10;
/// The size of a dot on screen, in pixels at the zoom it first shows.
const DOT_SIZE: f32 = 1.8;

/// How far into a fade a value is, from 0.0 at its first end to 1.0 at its second.
fn ramp(value: f32, (from, to): (f32, f32)) -> f32 {
    ((value - from) / (to - from)).clamp(0.0, 1.0)
}

/// Draws the dot grid across `visible`, a rectangle in editor coordinates. The dots sit at fixed
/// editor positions, so the grid pans and scales with the view. The minor dots fade out as the
/// view zooms away, then the major ones, so a far view has no dots too close to read.
pub(super) fn draw_grid(frame: &mut Frame, visible: Rectangle, zoom: f32) {
    let overall = ramp(zoom, GRID_FADE);
    if overall <= 0.0 {
        return;
    }
    let minor = overall * ramp(zoom, MINOR_FADE);
    // A dot is about the same size on screen at every zoom, within limits, so a far view stays
    // fine and a near view does not grow blobs.
    let size = (DOT_SIZE / zoom).clamp(DOT_SIZE * 0.6, DOT_SIZE * 1.8);

    let first_column = (visible.x / GRID_STEP).floor() as i64;
    let last_column = ((visible.x + visible.width) / GRID_STEP).ceil() as i64;
    let first_row = (visible.y / GRID_STEP).floor() as i64;
    let last_row = ((visible.y + visible.height) / GRID_STEP).ceil() as i64;

    let dot = |column: i64, row: i64| {
        (
            Point::new(
                column as f32 * GRID_STEP - size / 2.0,
                row as f32 * GRID_STEP - size / 2.0,
            ),
            Size::new(size, size),
        )
    };
    let is_major = |column: i64, row: i64| {
        column.rem_euclid(GRID_MAJOR_EVERY) == 0 && row.rem_euclid(GRID_MAJOR_EVERY) == 0
    };

    if minor > 0.0 {
        let minor_dots = Path::new(|builder| {
            for row in first_row..=last_row {
                for column in first_column..=last_column {
                    if !is_major(column, row) {
                        let (point, size) = dot(column, row);
                        builder.rectangle(point, size);
                    }
                }
            }
        });
        frame.fill(
            &minor_dots,
            Color::from_rgba(1.0, 1.0, 1.0, MINOR_ALPHA * minor),
        );
    }
    let major_dots = Path::new(|builder| {
        for row in first_row..=last_row {
            for column in first_column..=last_column {
                if is_major(column, row) {
                    let (point, size) = dot(column, row);
                    builder.rectangle(point, size);
                }
            }
        }
    });
    frame.fill(
        &major_dots,
        Color::from_rgba(1.0, 1.0, 1.0, MAJOR_ALPHA * overall),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fade_runs_from_zero_to_one_between_its_ends() {
        assert_eq!(ramp(0.1, (0.2, 0.4)), 0.0);
        assert!((ramp(0.3, (0.2, 0.4)) - 0.5).abs() < 1e-6);
        assert_eq!(ramp(0.9, (0.2, 0.4)), 1.0);
    }

    /// The minor dots are gone before the major ones, so the grid thins out as the view zooms away.
    #[test]
    fn the_minor_dots_go_before_the_major_ones() {
        assert!(MINOR_FADE.1 > GRID_FADE.1);
        assert!(MINOR_FADE.0 > GRID_FADE.0);
        // At the zoom where the minor dots start to show, the dots on screen are still far apart.
        assert!(GRID_STEP * MINOR_FADE.0 >= 14.0);
    }

    /// A shadow's layers get narrower toward the shape, and its total is the strength.
    #[test]
    fn a_shadow_adds_up_to_its_strength() {
        let alpha = NODE_SHADOW.strength / SHADOW_LAYERS as f32;
        let total = 1.0 - (1.0 - alpha).powi(SHADOW_LAYERS as i32);
        assert!((total - NODE_SHADOW.strength).abs() < 0.04, "{total}");
    }
}
