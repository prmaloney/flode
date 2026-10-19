//! The flode mark at the left of the pill: an F built from wired node cards.

use nih_plug_iced::widget::canvas::{Frame, LineCap, Path, Stroke};
use nih_plug_iced::{Color, Point, Rectangle};

use super::draw::rounded_rect;
use super::menus::pill_bounds;
use super::style::*;

/// The logo's side on screen, in pixels.
pub(super) const LOGO_SIZE: f32 = 30.0;
/// The gap between the window's left edge and the logo.
const LOGO_MARGIN: f32 = 16.0;
/// The least free space kept between the logo and the pill.
const LOGO_GAP: f32 = 12.0;

/// The side of the mark's design square, which the coordinates below are in.
const SVG_SIZE: f32 = 512.0;
const SVG_CORNER: f32 = 112.0;
/// A node card as `(x, y, width, height, corner, header height)`.
const STEM: (f32, f32, f32, f32, f32, f32) = (80.0, 70.0, 100.0, 372.0, 14.0, 22.0);
const SMALL_NODES: [(f32, f32, f32, f32, f32, f32); 3] = [
    (222.0, 70.0, 88.0, 58.0, 12.0, 18.0),
    (342.0, 70.0, 88.0, 58.0, 12.0, 18.0),
    (222.0, 190.0, 88.0, 58.0, 12.0, 18.0),
];
/// Each wire as `(start x, end x, y)`, with the color of its pins.
const WIRES: [(f32, f32, f32, Color); 3] = [
    (180.0, 222.0, 99.0, CONTROL_COLOR),
    (310.0, 342.0, 99.0, SELECTION_COLOR),
    (180.0, 222.0, 219.0, NOTES_COLOR),
];
const WIRE_WIDTH: f32 = 10.0;
const PIN_RADIUS: f32 = 9.0;
const PIN_OUTLINE: f32 = 5.0;

/// The tile behind the mark, the middle of the icon's gradient.
const LOGO_TILE_COLOR: Color = Color::from_rgb(0.114, 0.118, 0.161);
/// The dark ring around each pin.
const LOGO_PIN_OUTLINE_COLOR: Color = Color::from_rgb(0.078, 0.082, 0.106);

/// Where the logo goes, in screen coordinates: top-aligned with the pill, in the free space to its
/// left. It is `None` when that space is too small to hold the logo, so it never meets the pill
/// or the window edge.
pub(super) fn logo_bounds(bounds: Rectangle) -> Option<Rectangle> {
    let pill = pill_bounds(bounds);
    if LOGO_MARGIN + LOGO_SIZE + LOGO_GAP > pill.x {
        return None;
    }
    Some(Rectangle {
        x: LOGO_MARGIN,
        y: pill.y,
        width: LOGO_SIZE,
        height: LOGO_SIZE,
    })
}

/// Draws the logo into `rect`, scaling the icon's 512 unit square by one factor. It draws
/// shapes only, and it is the same on every frame.
pub(super) fn draw_logo(frame: &mut Frame, rect: Rectangle) {
    let k = rect.width / SVG_SIZE;
    let at = |x: f32, y: f32| Point::new(rect.x + x * k, rect.y + y * k);
    let card = |(x, y, w, h, _, _): (f32, f32, f32, f32, f32, f32)| Rectangle {
        x: rect.x + x * k,
        y: rect.y + y * k,
        width: w * k,
        height: h * k,
    };

    frame.fill(&rounded_rect(rect, SVG_CORNER * k), LOGO_TILE_COLOR);

    for node in std::iter::once(STEM).chain(SMALL_NODES) {
        let body = card(node);
        let corner = node.4 * k;
        frame.fill(&rounded_rect(body, corner), NODE_BODY_COLOR);
        // The header band has the card's rounded top corners and a square bottom edge.
        let (x0, y0) = (body.x, body.y);
        let (x1, y1) = (body.x + body.width, body.y + node.5 * k);
        let header = Path::new(|builder| {
            builder.move_to(Point::new(x0 + corner, y0));
            builder.line_to(Point::new(x1 - corner, y0));
            builder.quadratic_curve_to(Point::new(x1, y0), Point::new(x1, y0 + corner));
            builder.line_to(Point::new(x1, y1));
            builder.line_to(Point::new(x0, y1));
            builder.line_to(Point::new(x0, y0 + corner));
            builder.quadratic_curve_to(Point::new(x0, y0), Point::new(x0 + corner, y0));
            builder.close();
        });
        frame.fill(&header, NODE_HEADER_COLOR);
    }

    for (start, end, y, color) in WIRES {
        frame.stroke(
            &Path::line(at(start, y), at(end, y)),
            Stroke::default()
                .with_color(color)
                .with_width(WIRE_WIDTH * k)
                .with_line_cap(LineCap::Round),
        );
        for x in [start, end] {
            let pin = Path::circle(at(x, y), PIN_RADIUS * k);
            frame.fill(&pin, color);
            frame.stroke(
                &pin,
                Stroke::default()
                    .with_color(LOGO_PIN_OUTLINE_COLOR)
                    .with_width(PIN_OUTLINE * k),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(width: f32) -> Rectangle {
        Rectangle {
            x: 0.0,
            y: 0.0,
            width,
            height: 712.0,
        }
    }

    #[test]
    fn logo_fits_the_editor_and_clears_the_pill() {
        let bounds = window(1024.0);
        let logo = logo_bounds(bounds).expect("fits at 1024");
        let pill = pill_bounds(bounds);
        assert_eq!(logo.y, pill.y);
        assert!(logo.x >= 0.0);
        assert!(logo.x + logo.width < pill.x);
    }

    #[test]
    fn logo_hides_when_the_window_is_too_narrow() {
        assert!(logo_bounds(window(PILL_WIDTH)).is_none());
        assert!(logo_bounds(window(100.0)).is_none());
    }

    #[test]
    fn logo_never_overlaps_the_pill() {
        let mut width = 0.0;
        while width < 1600.0 {
            let bounds = window(width);
            if let Some(logo) = logo_bounds(bounds) {
                let pill = pill_bounds(bounds);
                assert!(logo.x >= 0.0 && logo.x + logo.width <= pill.x, "{width}");
            }
            width += 7.0;
        }
    }
}
