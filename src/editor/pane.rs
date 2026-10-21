//! Drawing for an open group: the dimmed backdrop, the pane's frame and title strip, and the box
//! that renames. Where things sit is in `group_ui`.

use nih_plug_iced::widget::canvas::{Frame, LineCap, Path, Stroke, Text};
use nih_plug_iced::{alignment, Color, Point, Rectangle};

use super::depth::{draw_shadow, Shadow};
use super::draw::{draw_text, rounded_rect};
use super::graph::Graph;
use super::group_ui::{
    close_bounds, rename_bounds, save_bounds, title_bounds, title_layout, TITLE_HEIGHT,
};
use super::style::*;

/// The dimming over the editor behind the pane.
const BACKDROP_COLOR: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.5);
/// The pane's own ground, the same as the window's, so a graph looks the same inside it.
const PANE_COLOR: Color = BACKGROUND_COLOR;
/// The pane floats higher than a menu, so its shadow reaches further.
const PANE_SHADOW: Shadow = Shadow {
    reach: 30.0,
    drop: 8.0,
    strength: 0.6,
};
const PANE_RADIUS: f32 = 12.0;
const PANE_BORDER_COLOR: Color = Color::from_rgba(0.42, 0.68, 1.0, 0.45);
const CLOSE_HOVER_COLOR: Color = Color::from_rgb(0.30, 0.31, 0.38);

/// Draws the backdrop over the editor, then the pane's shadow, ground, border, and title strip
/// with the breadcrumb and the close button. The pane's graph is drawn over this in a layer of
/// its own.
pub(super) fn draw_pane(frame: &mut Frame, bounds: Rectangle, graph: &Graph, pane: Rectangle) {
    frame.fill_rectangle(Point::ORIGIN, bounds.size(), BACKDROP_COLOR);
    draw_shadow(frame, pane, PANE_RADIUS, PANE_SHADOW);
    frame.fill(&rounded_rect(pane, PANE_RADIUS), PANE_COLOR);

    // The title strip is a rounded rectangle with its bottom corners squared off, like a node's
    // header.
    let title = title_bounds(pane);
    frame.fill(&rounded_rect(title, PANE_RADIUS), PILL_COLOR);
    frame.fill(
        &Path::rectangle(
            Point::new(title.x, title.y + title.height - PANE_RADIUS),
            nih_plug_iced::Size::new(title.width, PANE_RADIUS),
        ),
        PILL_COLOR,
    );
    frame.stroke(
        &Path::line(
            Point::new(title.x, title.y + title.height),
            Point::new(title.x + title.width, title.y + title.height),
        ),
        Stroke::default()
            .with_color(PILL_DIVIDER_COLOR)
            .with_width(1.0),
    );
    frame.stroke(
        &rounded_rect(pane, PANE_RADIUS),
        Stroke::default()
            .with_color(PANE_BORDER_COLOR)
            .with_width(1.5),
    );

    let layout = title_layout(pane, &graph.group_path_names());
    let middle = pane.y + TITLE_HEIGHT / 2.0;
    let mut text = |content: String, x: f32, color: Color| {
        frame.fill_text(Text {
            content,
            position: Point::new(x, middle),
            color,
            size: 13.0,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
    };
    text(layout.prefix.clone(), pane.x + 14.0, DIM_TEXT_COLOR);
    text(layout.name.clone(), layout.name_x, TEXT_COLOR);

    // The save button is a small floppy disk, lit while the cursor is over it.
    let save = save_bounds(pane);
    if graph.cursor_in && save.contains(graph.last_screen) {
        frame.fill(
            &rounded_rect(save, DROPDOWN_CORNER_RADIUS),
            CLOSE_HOVER_COLOR,
        );
    }
    let (left, top, right, bottom) = (
        save.x + 4.5,
        save.y + 4.5,
        save.x + save.width - 4.5,
        save.y + save.height - 4.5,
    );
    let disk = Path::new(|builder| {
        builder.move_to(Point::new(left, top));
        builder.line_to(Point::new(right - 3.0, top));
        builder.line_to(Point::new(right, top + 3.0));
        builder.line_to(Point::new(right, bottom));
        builder.line_to(Point::new(left, bottom));
        builder.close();
    });
    let line = Stroke::default()
        .with_color(TEXT_COLOR)
        .with_width(1.3)
        .with_line_cap(LineCap::Round);
    frame.stroke(&disk, line);
    // The shutter on top and the label below it.
    frame.stroke(
        &Path::rectangle(
            Point::new(left + 3.0, top),
            nih_plug_iced::Size::new(right - left - 7.0, 3.5),
        ),
        line,
    );
    frame.stroke(
        &Path::rectangle(
            Point::new(left + 2.5, bottom - 5.5),
            nih_plug_iced::Size::new(right - left - 5.0, 5.5),
        ),
        line,
    );

    // The close button is a square with a cross, lit while the cursor is over it.
    let close = close_bounds(pane);
    if graph.cursor_in && close.contains(graph.last_screen) {
        frame.fill(
            &rounded_rect(close, DROPDOWN_CORNER_RADIUS),
            CLOSE_HOVER_COLOR,
        );
    }
    let center = Point::new(close.x + close.width / 2.0, close.y + close.height / 2.0);
    let reach = 4.5;
    let cross = Stroke::default()
        .with_color(TEXT_COLOR)
        .with_width(1.5)
        .with_line_cap(LineCap::Round);
    frame.stroke(
        &Path::line(
            Point::new(center.x - reach, center.y - reach),
            Point::new(center.x + reach, center.y + reach),
        ),
        cross,
    );
    frame.stroke(
        &Path::line(
            Point::new(center.x - reach, center.y + reach),
            Point::new(center.x + reach, center.y - reach),
        ),
        cross,
    );
}

/// The rename box: the name with a caret in a field, over a shadow. The name is on a lighter
/// ground while it is still selected, so it reads as the thing the next key replaces.
pub(super) fn draw_rename(frame: &mut Frame, bounds: Rectangle, graph: &Graph) {
    let Some(rename) = &graph.rename else {
        return;
    };
    let rect = rename_bounds(bounds, rename.screen);
    draw_shadow(
        frame,
        rect,
        DROPDOWN_CORNER_RADIUS,
        super::depth::FLOAT_SHADOW,
    );
    frame.fill(&rounded_rect(rect, DROPDOWN_CORNER_RADIUS), DROPDOWN_COLOR);
    frame.stroke(
        &rounded_rect(rect, DROPDOWN_CORNER_RADIUS),
        Stroke::default()
            .with_color(SELECTION_COLOR)
            .with_width(1.0),
    );
    if rename.fresh {
        let width = (rename.text.chars().count() as f32 * 7.0 + 8.0).min(rect.width - 12.0);
        let band = Rectangle {
            x: rect.x + 6.0,
            y: rect.y + 4.0,
            width,
            height: rect.height - 8.0,
        };
        frame.fill(
            &rounded_rect(band, 3.0),
            Color {
                a: 0.45,
                ..SELECTION_COLOR
            },
        );
    }
    let content = if rename.fresh {
        rename.text.clone()
    } else {
        format!("{}|", rename.text)
    };
    draw_text(
        frame,
        &[],
        1.0,
        Text {
            content,
            position: Point::new(rect.x + 10.0, rect.y + rect.height / 2.0),
            color: TEXT_COLOR,
            size: 13.0,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        },
    );
}
