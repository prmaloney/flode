//! A rotary knob. The node editor draws the knob into its own canvas and decides which mouse
//! events reach it. This module turns those events into a new normalized value, which the node
//! stores. It keeps the drag state and detects double clicks.

use nih_plug_iced::widget::canvas::{path::Arc, Frame, LineCap, Path, Stroke};
use nih_plug_iced::{mouse, Color, Point};
use std::f32::consts::PI;

/// Vertical drag distance that sweeps the full range.
const DRAG_PIXELS_FOR_FULL_RANGE: f32 = 200.0;

/// Clock angles, where 0 points at 12 o'clock and angles increase clockwise.
/// The knob sweeps 270°, starting at 7:30.
const START_ANGLE: f32 = -PI * 0.75;
const SWEEP: f32 = PI * 1.5;

/// Radii and widths as fractions of the knob diameter.
const TRACK_RADIUS: f32 = 0.44;
const BODY_RADIUS: f32 = 0.34;
const POINTER_INNER: f32 = 0.05;
const POINTER_OUTER: f32 = 0.30;
const ARC_WIDTH: f32 = 0.05;
const POINTER_WIDTH: f32 = 0.03;

const BODY_COLOR: Color = Color::from_rgb(0.205, 0.21, 0.255);
const TRACK_COLOR: Color = Color::from_rgb(0.35, 0.36, 0.42);
const FILL_COLOR: Color = Color::from_rgb(0.19, 0.50, 0.90);
const POINTER_COLOR: Color = Color::WHITE;
/// A knob driven by a control wire is greyed out, and the dot shows the live signal.
const DISABLED_TRACK_COLOR: Color = Color::from_rgb(0.26, 0.26, 0.28);
const DISABLED_FILL_COLOR: Color = Color::from_rgb(0.40, 0.40, 0.42);
const DISABLED_POINTER_COLOR: Color = Color::from_rgb(0.50, 0.50, 0.52);
const INDICATOR_COLOR: Color = Color::from_rgb(0.35, 0.85, 0.45);

/// Mouse state that survives between events.
#[derive(Debug, Default)]
pub struct State {
    /// Cursor y position and normalized value when the drag started. `None` when not dragging.
    drag_start: Option<(f32, f32)>,
    /// Used to detect double clicks.
    last_click: Option<mouse::Click>,
}

/// True if `point` falls inside the knob centered on `center`.
pub fn contains(center: Point, diameter: f32, point: Point) -> bool {
    let radius = diameter / 2.0;
    let dx = point.x - center.x;
    let dy = point.y - center.y;
    dx * dx + dy * dy <= radius * radius
}

/// What a press on a knob does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// Start dragging from the current value.
    Drag,
    /// A double click. Reset the value to its default.
    Reset,
}

/// Starts a drag from `value`, or reports a reset when the press is a double click. The caller
/// decides what a reset sets the value to.
pub fn press(state: &mut State, value: f32, position: Point) -> Press {
    let click = mouse::Click::new(position, state.last_click);
    state.last_click = Some(click);

    if matches!(click.kind(), mouse::click::Kind::Double) {
        state.drag_start = None;
        return Press::Reset;
    }

    state.drag_start = Some((position.y, value));
    Press::Drag
}

/// Records a press at `position` in `last`, and returns true if it is the second press of a double
/// click. A double click clears `last`, so the next press starts a new one. For presses that need
/// no knob state, such as the clicks on an LFO's drawn curve.
pub fn double_click(last: &mut Option<mouse::Click>, position: Point) -> bool {
    let click = mouse::Click::new(position, *last);
    let double = matches!(click.kind(), mouse::click::Kind::Double);
    *last = (!double).then_some(click);
    double
}

/// The normalized value for the cursor during a drag. Dragging up increases the value. None when
/// no drag is active.
pub fn drag(state: &State, position: Point) -> Option<f32> {
    let (start_y, start_value) = state.drag_start?;
    let delta = (start_y - position.y) / DRAG_PIXELS_FOR_FULL_RANGE;
    Some((start_value + delta).clamp(0.0, 1.0))
}

/// Ends a drag.
pub fn release(state: &mut State) {
    state.drag_start = None;
}

/// Draws the knob centered on `center`. `value` is normalized from 0.0 to 1.0.
pub fn draw(frame: &mut Frame, center: Point, diameter: f32, value: f32, disabled: bool) {
    let (track_color, fill_color, pointer_color) = if disabled {
        (
            DISABLED_TRACK_COLOR,
            DISABLED_FILL_COLOR,
            DISABLED_POINTER_COLOR,
        )
    } else {
        (TRACK_COLOR, FILL_COLOR, POINTER_COLOR)
    };
    let value_angle = START_ANGLE + SWEEP * value;
    let track_radius = diameter * TRACK_RADIUS;

    // Body circle behind the arc and pointer.
    frame.fill(&Path::circle(center, diameter * BODY_RADIUS), BODY_COLOR);

    // Unfilled track across the full sweep.
    let track = Path::new(|builder| {
        builder.arc(Arc {
            center,
            radius: track_radius,
            start_angle: iced_angle(START_ANGLE),
            end_angle: iced_angle(START_ANGLE + SWEEP),
        });
    });
    frame.stroke(&track, arc_stroke(track_color, diameter));

    // Filled arc from the start of the sweep up to the current value.
    if value > 0.0 {
        let filled = Path::new(|builder| {
            builder.arc(Arc {
                center,
                radius: track_radius,
                start_angle: iced_angle(START_ANGLE),
                end_angle: iced_angle(value_angle),
            });
        });
        frame.stroke(&filled, arc_stroke(fill_color, diameter));
    }

    // The pointer is a bar drawn straight up, rotated about the center. Frame::fill_rectangle
    // only transforms the corner point, so the bar is filled as a path, which takes the full
    // rotation.
    frame.with_save(|frame| {
        frame.translate(nih_plug_iced::Vector::new(center.x, center.y));
        frame.rotate(value_angle);
        let pointer = Path::rectangle(
            Point::new(-diameter * POINTER_WIDTH / 2.0, -diameter * POINTER_OUTER),
            nih_plug_iced::Size::new(
                diameter * POINTER_WIDTH,
                diameter * (POINTER_OUTER - POINTER_INNER),
            ),
        );
        frame.fill(&pointer, pointer_color);
    });
    if disabled {
        // The dot sits on the track at the live value.
        let angle = iced_angle(value_angle);
        let dot = Point::new(
            center.x + track_radius * angle.cos(),
            center.y + track_radius * angle.sin(),
        );
        frame.fill(&Path::circle(dot, diameter * 0.07), INDICATOR_COLOR);
    }
}

/// Converts a clock angle (0 at 12 o'clock) to an iced canvas angle (0 at 3 o'clock).
fn iced_angle(clock_angle: f32) -> f32 {
    clock_angle - PI / 2.0
}

fn arc_stroke<'a>(color: Color, diameter: f32) -> Stroke<'a> {
    Stroke::default()
        .with_color(color)
        .with_width(diameter * ARC_WIDTH)
        .with_line_cap(LineCap::Round)
}
