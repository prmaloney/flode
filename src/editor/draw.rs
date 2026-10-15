//! Everything the editor paints: wires, nodes, knobs, dropdowns, the pill, and the lists.

use crate::editor::knob::{self};
use crate::editor::piano::{self};
use crate::nodes::map;
use crate::nodes::{
    pin_y, Category, FlodeNode, Port, ARROW_CENTER_X, HEADER_HEIGHT, NODE_WIDTH, ROW_KNOB_DIAMETER,
    TITLE_INSET,
};
use crate::patch::control::ControlLevels;
use nih_plug_iced::widget::canvas::{Frame, Geometry, LineCap, Path, Stroke, Text};
use nih_plug_iced::{alignment, Color, Point, Rectangle};

use super::{curve::*, graph::*, menus::*, style::*};

/// Draws one frame of the editor: the wires, the nodes, then the pill, key strip, and open
/// lists on top.
pub(super) fn draw_editor(
    graph: &Graph,
    levels: ControlLevels,
    active_notes: [u64; 2],
    output: [f32; 2],
    bounds: Rectangle,
) -> Vec<Geometry> {
    let mut frame = Frame::new(bounds.size());
    let view = graph.view;
    let offset = view.offset;

    // Iced draws all text above all shapes in a frame. Node text under an open overlay would
    // show through it, so node text is hidden inside the overlays. The overlays are kept in
    // editor coordinates. The pill is in screen coordinates, so it is converted.
    // The key strip covers nodes too, so its area is an overlay as well.
    let mut overlays = vec![
        view.rect_to_world(pill_bounds(bounds)),
        view.rect_to_world(piano::strip_bounds(bounds)),
    ];
    if let Some((index, which)) = graph.dropdown_menu {
        let node = &graph.nodes[index];
        if let Some(dropdown) = node.dropdowns().into_iter().find(|d| d.which == which) {
            // The pin labels sit left of the list, so the cover spans the node's full width.
            let list = dropdown_list_bounds(dropdown.bounds, dropdown.labels.len());
            overlays.push(Rectangle {
                x: node.position.x,
                y: list.y,
                width: NODE_WIDTH,
                height: list.height,
            });
        }
    }
    if let Some(menu) = graph.menu {
        overlays.push(menu_bounds(menu));
    }
    if let Some(category) = graph.category_menu {
        overlays.push(view.rect_to_world(category_list_bounds(bounds, category)));
    }

    // The nodes and wires are drawn in editor coordinates, then the view scales and moves them.
    frame.with_save(|frame| {
        frame.translate(offset);
        frame.scale(view.zoom);

        // Ordinary wires go under the nodes. The wire a dragged node would splice into, and the
        // wire being dragged, go over them so they stay visible.
        for (index, connection) in graph.connections.iter().enumerate() {
            if graph.link_target == Some(index) {
                continue;
            }
            if let Some(curve) = graph.link_curve(connection) {
                draw_curve(frame, &curve, wire_color(connection.out), 2.0);
            }
        }

        // Iced draws all text above all shapes in a frame, so a node drawn later would still
        // have the text of the nodes under it showing through. Each node's text is hidden inside
        // the nodes drawn after it, which also puts the selected node, drawn last, on top.
        let order = graph.draw_order();
        for (position, &index) in order.iter().enumerate() {
            let mut covered = overlays.clone();
            covered.extend(
                order[position + 1..]
                    .iter()
                    .map(|&above| graph.nodes[above].bounds()),
            );
            draw_node(frame, index, graph, levels, &covered);
        }

        if let Some(index) = graph.link_target {
            if let Some(curve) = graph.link_curve(&graph.connections[index]) {
                draw_curve(frame, &curve, HIGHLIGHT_WIRE_COLOR, 4.0);
            }
        }

        if let Interaction::Wiring { from, out, cursor } = graph.interaction {
            if let Some(start) = graph.nodes[from].output_point(out) {
                let color = if out == Port::Audio {
                    PENDING_WIRE_COLOR
                } else {
                    CONTROL_COLOR
                };
                draw_curve(frame, &wire_curve(start, cursor), color, 2.0);
            }
        }

        if let Some(index) = graph.selected {
            draw_selection(frame, &graph.nodes[index]);
        }
    });

    // The pill stays fixed on screen. It is drawn after the nodes, so it covers them.
    draw_pill(&mut frame, bounds);
    super::meter::draw(&mut frame, bounds, output);

    let strip = piano::strip_bounds(bounds);
    piano::draw_strip(&mut frame, strip, active_notes, graph.mouse_note);

    // The open lists and the menu are drawn over the pill, in editor coordinates like the nodes.
    frame.with_save(|frame| {
        frame.translate(offset);
        frame.scale(view.zoom);

        if let Some((index, which)) = graph.dropdown_menu {
            let node = &graph.nodes[index];
            if let Some(dropdown) = node.dropdowns().into_iter().find(|d| d.which == which) {
                let list = dropdown_list_bounds(dropdown.bounds, dropdown.labels.len());
                let hovered_row = match graph.hover {
                    Some(Hover::Row {
                        node,
                        which: hovered,
                        row,
                    }) if node == index && hovered == which => Some(row),
                    _ => None,
                };
                draw_list(
                    frame,
                    list,
                    &dropdown.labels,
                    dropdown.selected,
                    hovered_row,
                    view.zoom,
                );
            }
        }

        if let Some(menu) = graph.menu {
            if let Some(node) = graph.nodes.get(menu.node) {
                draw_menu(frame, menu, node, view.zoom);
            }
        }
    });

    if let Some(category) = graph.category_menu {
        draw_category_list(&mut frame, bounds, category);
    }

    vec![frame.into_geometry()]
}

/// Audio is grey. Every other signal is green.
pub(super) fn wire_color(out: Port) -> Color {
    if out == Port::Audio {
        WIRE_COLOR
    } else {
        CONTROL_COLOR
    }
}

/// The four points of a wire's cubic curve. Control points stay at least 60 pixels out so short
/// wires still curve.
pub(super) fn wire_curve(from: Point, to: Point) -> [Point; 4] {
    let reach = ((to.x - from.x).abs() * 0.5).max(60.0);
    [
        from,
        Point::new(from.x + reach, from.y),
        Point::new(to.x - reach, to.y),
        to,
    ]
}

pub(super) fn bezier_point(curve: &[Point; 4], t: f32) -> Point {
    let u = 1.0 - t;
    let a = u * u * u;
    let b = 3.0 * u * u * t;
    let c = 3.0 * u * t * t;
    let d = t * t * t;
    Point::new(
        a * curve[0].x + b * curve[1].x + c * curve[2].x + d * curve[3].x,
        a * curve[0].y + b * curve[1].y + c * curve[2].y + d * curve[3].y,
    )
}

pub(super) fn draw_curve(frame: &mut Frame, curve: &[Point; 4], color: Color, width: f32) {
    let [p0, p1, p2, p3] = *curve;
    let path = Path::new(|builder| {
        builder.move_to(p0);
        builder.bezier_curve_to(p1, p2, p3);
    });
    frame.stroke(
        &path,
        Stroke::default()
            .with_color(color)
            .with_width(width)
            .with_line_cap(LineCap::Round),
    );
}

pub(super) fn draw_node(
    frame: &mut Frame,
    index: usize,
    graph: &Graph,
    levels: ControlLevels,
    covered: &[Rectangle],
) {
    let node = &graph.nodes[index];
    let zoom = graph.view.zoom;
    let bounds = node.bounds();
    // A muted or bypassed node is darker, and says which in its header.
    let off = node.bypass_label();
    let (body_color, header_color) = if off.is_some() {
        (NODE_OFF_BODY_COLOR, NODE_OFF_HEADER_COLOR)
    } else {
        (NODE_BODY_COLOR, NODE_HEADER_COLOR)
    };
    frame.fill(&rounded_rect(bounds, CORNER_RADIUS), body_color);

    // The header is a rounded rectangle with its bottom corners squared off.
    let header = node.header_bounds();
    frame.fill(&rounded_rect(header, CORNER_RADIUS), header_color);
    frame.fill(
        &Path::rectangle(
            Point::new(header.x, header.y + header.height - CORNER_RADIUS),
            nih_plug_iced::Size::new(header.width, CORNER_RADIUS),
        ),
        header_color,
    );

    draw_text(
        frame,
        covered,
        zoom,
        Text {
            content: node.title().to_string(),
            position: Point::new(header.x + TITLE_INSET, header.y + HEADER_HEIGHT / 2.0),
            color: if off.is_some() {
                DIM_TEXT_COLOR
            } else {
                TEXT_COLOR
            },
            size: 14.0,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        },
    );
    if let Some(label) = off {
        draw_text(
            frame,
            covered,
            zoom,
            Text {
                content: label.to_string(),
                position: Point::new(
                    header.x + header.width - 10.0,
                    header.y + HEADER_HEIGHT / 2.0,
                ),
                color: DIM_TEXT_COLOR,
                size: 10.0,
                horizontal_alignment: alignment::Horizontal::Right,
                vertical_alignment: alignment::Vertical::Center,
                ..Text::default()
            },
        );
    }

    draw_collapse_arrow(frame, node);

    // A collapsed node shows only its header. Its pins are dots at the header's edges, and wires
    // still attach to them.
    if node.collapsed {
        for port in node.input_ports() {
            if let Some(point) = node.input_point(port) {
                draw_pin_dot(frame, point, port);
            }
        }
        for port in node.output_ports() {
            if let Some(point) = node.output_point(port) {
                draw_pin_dot(frame, point, port);
            }
        }
        return;
    }

    // Each row has an optional input pin on the left edge, a knob with its name and value, and an
    // optional output pin on the right edge.
    for (row_index, row) in node.rows().iter().enumerate() {
        let y = node.position.y + pin_y(row_index);
        let left = node.position.x;
        let right = node.position.x + NODE_WIDTH;

        if let Some(id) = row.knob {
            let driven = graph.is_driven(index, id);
            let text_color = if driven { DIM_TEXT_COLOR } else { TEXT_COLOR };
            draw_text(
                frame,
                covered,
                zoom,
                Text {
                    content: id.name().to_string(),
                    position: Point::new(left + PIN_LABEL_INSET, y + NAME_OFFSET),
                    color: text_color,
                    size: PIN_LABEL_SIZE,
                    vertical_alignment: alignment::Vertical::Center,
                    ..Text::default()
                },
            );
            draw_text(
                frame,
                covered,
                zoom,
                Text {
                    content: id.format(node.value(id)),
                    position: Point::new(left + PIN_LABEL_INSET, y + VALUE_OFFSET),
                    color: text_color,
                    size: PIN_LABEL_SIZE,
                    vertical_alignment: alignment::Vertical::Center,
                    ..Text::default()
                },
            );
        }

        if let Some(port) = row.input {
            let point = Point::new(left, y);
            draw_pin_dot(frame, point, port);
            // A pin with no knob to name it draws its own label, such as "Shape" or "A".
            if row.knob.is_none() {
                if let Some(label) = port.input_label() {
                    draw_text(
                        frame,
                        covered,
                        zoom,
                        Text {
                            content: label.to_string(),
                            position: Point::new(point.x + PIN_LABEL_INSET, point.y),
                            color: TEXT_COLOR,
                            size: PIN_LABEL_SIZE,
                            vertical_alignment: alignment::Vertical::Center,
                            ..Text::default()
                        },
                    );
                }
            }
        }

        if let Some(port) = row.output {
            let point = Point::new(right, y);
            draw_pin_dot(frame, point, port);
            if let Some(label) = port.output_label() {
                draw_text(
                    frame,
                    covered,
                    zoom,
                    Text {
                        content: label.to_string(),
                        position: Point::new(point.x - PIN_LABEL_INSET, point.y),
                        color: TEXT_COLOR,
                        size: PIN_LABEL_SIZE,
                        horizontal_alignment: alignment::Horizontal::Right,
                        vertical_alignment: alignment::Vertical::Center,
                        ..Text::default()
                    },
                );
            }
        }
    }

    {
        let labels = node.switch_labels();
        for (index, label) in labels.iter().enumerate() {
            draw_switch(frame, node, index, label, covered, zoom);
        }
    }

    for dropdown in node.dropdowns() {
        let hovered = graph.hover
            == Some(Hover::Button {
                node: index,
                which: dropdown.which,
            });
        draw_dropdown(
            frame,
            dropdown.bounds,
            dropdown.name,
            dropdown.labels[dropdown.selected],
            hovered,
            zoom,
            covered,
        );
    }

    if node.has_curve() {
        draw_map_curve(frame, index, graph, levels);
    }

    for slot in node.knob_slots() {
        let route = if slot.pinned {
            graph.control_route(index, slot.id)
        } else {
            None
        };
        // A driven knob shows the value its control route sets for the live signal. Its own value
        // is ignored while it's driven.
        let value = match route {
            Some(route) => route.apply(&levels),
            None => node.value(slot.id),
        };
        knob::draw(
            frame,
            slot.center,
            ROW_KNOB_DIAMETER,
            value,
            route.is_some(),
        );
    }
}

/// The arrow in the header. It points down when the node is open and right when it is collapsed.
pub(super) fn draw_collapse_arrow(frame: &mut Frame, node: &FlodeNode) {
    let x = node.position.x + ARROW_CENTER_X;
    let y = node.position.y + HEADER_HEIGHT / 2.0;
    let path = if node.collapsed {
        Path::new(|builder| {
            builder.move_to(Point::new(x - 2.5, y - 4.0));
            builder.line_to(Point::new(x + 3.0, y));
            builder.line_to(Point::new(x - 2.5, y + 4.0));
            builder.close();
        })
    } else {
        Path::new(|builder| {
            builder.move_to(Point::new(x - 4.0, y - 2.5));
            builder.line_to(Point::new(x + 4.0, y - 2.5));
            builder.line_to(Point::new(x, y + 3.0));
            builder.close();
        })
    };
    frame.fill(&path, TEXT_COLOR);
}

/// A pin without its label. Control pins are green and audio pins are grey.
pub(super) fn draw_pin_dot(frame: &mut Frame, point: Point, port: Port) {
    if port.is_audio() {
        frame.fill(&Path::circle(point, PIN_RADIUS), TERMINAL_COLOR);
    } else {
        frame.fill(&Path::circle(point, CONTROL_PIN_RADIUS), CONTROL_COLOR);
    }
}

/// Draws node text unless its position is inside an overlay that covers the node.
pub(super) fn draw_text(frame: &mut Frame, covered: &[Rectangle], zoom: f32, mut text: Text) {
    let area = text_area(&text);
    text.size *= zoom;
    if !covered.iter().any(|rect| overlaps(rect, &area)) {
        frame.fill_text(text);
    }
}

/// The rectangle a piece of text takes, roughly, in the units of its position and size. The font
/// is proportional, so the width is an estimate from the character count. A label that touches a
/// covering rectangle is hidden whole, so no half of it shows through.
pub(super) fn text_area(text: &Text) -> Rectangle {
    const CHARACTER_WIDTH: f32 = 0.55;
    const LINE_HEIGHT: f32 = 1.2;
    let width = text.content.chars().count() as f32 * text.size * CHARACTER_WIDTH;
    let height = text.size * LINE_HEIGHT;
    let x = match text.horizontal_alignment {
        alignment::Horizontal::Left => text.position.x,
        alignment::Horizontal::Center => text.position.x - width / 2.0,
        alignment::Horizontal::Right => text.position.x - width,
    };
    let y = match text.vertical_alignment {
        alignment::Vertical::Top => text.position.y,
        alignment::Vertical::Center => text.position.y - height / 2.0,
        alignment::Vertical::Bottom => text.position.y - height,
    };
    Rectangle {
        x,
        y,
        width,
        height,
    }
}

/// True if the two rectangles share any area.
pub(super) fn overlaps(a: &Rectangle, b: &Rectangle) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/// The transfer curve of a map: the mapping line, its two handles, and a dot where the live input
/// lands on the curve.
pub(super) fn draw_map_curve(
    frame: &mut Frame,
    index: usize,
    graph: &Graph,
    levels: ControlLevels,
) {
    let node = &graph.nodes[index];
    let bounds = node.curve_bounds();
    frame.fill(
        &rounded_rect(bounds, DROPDOWN_CORNER_RADIUS),
        DROPDOWN_COLOR,
    );

    let stage = map::stage(node);
    let line = Path::new(|builder| {
        for i in 0..=CURVE_SAMPLES {
            let x = i as f32 / CURVE_SAMPLES as f32;
            let y = stage.apply(x);
            let point = Point::new(
                bounds.x + x * bounds.width,
                bounds.y + (1.0 - y) * bounds.height,
            );
            if i == 0 {
                builder.move_to(point);
            } else {
                builder.line_to(point);
            }
        }
    });
    frame.stroke(
        &line,
        Stroke::default()
            .with_color(TEXT_COLOR)
            .with_width(1.5)
            .with_line_cap(LineCap::Round),
    );

    for handle in map_handles(node) {
        frame.fill(&Path::circle(handle, 4.0), TEXT_COLOR);
    }

    if let Some(input) = graph.map_input_level(index, &levels) {
        let output = stage.apply(input);
        let dot = Point::new(
            bounds.x + input * bounds.width,
            bounds.y + (1.0 - output) * bounds.height,
        );
        frame.fill(&Path::circle(dot, 3.5), piano::SOUNDING_DOT_COLOR);
    }
}

/// A switch: a box with a check when on, and its label. The output's "Always" is dimmed while
/// mono is off, since it has no effect then.
pub(super) fn draw_switch(
    frame: &mut Frame,
    node: &FlodeNode,
    index: usize,
    label: &str,
    covered: &[Rectangle],
    zoom: f32,
) {
    let row = node.switch_bounds(index);
    let enabled = index == 0 || node.switch(0);
    let text_color = if enabled { TEXT_COLOR } else { DIM_TEXT_COLOR };
    let box_x = row.x + 2.0;
    let box_y = row.y + (row.height - CHECKBOX_SIZE) / 2.0;
    let outline = Rectangle {
        x: box_x,
        y: box_y,
        width: CHECKBOX_SIZE,
        height: CHECKBOX_SIZE,
    };
    frame.fill(&rounded_rect(outline, 2.0), DROPDOWN_COLOR);
    if node.switch(index) {
        frame.fill(
            &rounded_rect(
                Rectangle {
                    x: box_x + 3.0,
                    y: box_y + 3.0,
                    width: CHECKBOX_SIZE - 6.0,
                    height: CHECKBOX_SIZE - 6.0,
                },
                1.0,
            ),
            CHECKBOX_CHECKED_COLOR,
        );
    }
    draw_text(
        frame,
        covered,
        zoom,
        Text {
            content: label.to_string(),
            position: Point::new(box_x + CHECKBOX_SIZE + 8.0, row.y + row.height / 2.0),
            color: text_color,
            size: 13.0,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        },
    );
}

/// The closed dropdown: the current label and a chevron.
pub(super) fn draw_dropdown(
    frame: &mut Frame,
    bounds: Rectangle,
    name: &str,
    label: &str,
    hovered: bool,
    zoom: f32,
    covered: &[Rectangle],
) {
    let color = if hovered {
        DROPDOWN_HOVER_COLOR
    } else {
        DROPDOWN_COLOR
    };
    frame.fill(&rounded_rect(bounds, DROPDOWN_CORNER_RADIUS), color);
    // The labels go through `draw_text`, so an open list over this node hides them. The name sits
    // dimmed on the left, and the chosen value sits right of centre, before the chevron.
    draw_text(
        frame,
        covered,
        zoom,
        Text {
            content: name.to_string(),
            position: Point::new(bounds.x + 8.0, bounds.y + bounds.height / 2.0),
            color: DIM_TEXT_COLOR,
            size: 11.0,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        },
    );
    draw_text(
        frame,
        covered,
        zoom,
        Text {
            content: label.to_string(),
            position: Point::new(
                bounds.x + bounds.width - 22.0,
                bounds.y + bounds.height / 2.0,
            ),
            color: TEXT_COLOR,
            size: 13.0,
            horizontal_alignment: alignment::Horizontal::Right,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        },
    );

    let chevron_x = bounds.x + bounds.width - 12.0;
    let chevron_y = bounds.y + bounds.height / 2.0;
    let chevron = Path::new(|builder| {
        builder.move_to(Point::new(chevron_x - 4.0, chevron_y - 2.0));
        builder.line_to(Point::new(chevron_x, chevron_y + 2.0));
        builder.line_to(Point::new(chevron_x + 4.0, chevron_y - 2.0));
    });
    frame.stroke(
        &chevron,
        Stroke::default()
            .with_color(TEXT_COLOR)
            .with_width(1.5)
            .with_line_cap(LineCap::Round),
    );
}

/// The open dropdown list. The selected row is highlighted.
pub(super) fn draw_list(
    frame: &mut Frame,
    list: Rectangle,
    labels: &[&str],
    selected: usize,
    hovered_row: Option<usize>,
    zoom: f32,
) {
    frame.fill(&rounded_rect(list, MENU_CORNER_RADIUS), MENU_COLOR);

    for (row, label) in labels.iter().enumerate() {
        let row_y = list.y + row as f32 * WAVEFORM_ITEM_HEIGHT;
        if hovered_row == Some(row) {
            frame.fill(
                &rounded_rect(
                    Rectangle {
                        x: list.x + 4.0,
                        y: row_y + 2.0,
                        width: list.width - 8.0,
                        height: WAVEFORM_ITEM_HEIGHT - 4.0,
                    },
                    4.0,
                ),
                LIST_ROW_HOVER_COLOR,
            );
        }
        if row == selected {
            frame.fill(
                &rounded_rect(
                    Rectangle {
                        x: list.x + 4.0,
                        y: row_y + 2.0,
                        width: list.width - 8.0,
                        height: WAVEFORM_ITEM_HEIGHT - 4.0,
                    },
                    4.0,
                ),
                NODE_HEADER_COLOR,
            );
        }
        frame.fill_text(Text {
            content: label.to_string(),
            position: Point::new(list.x + 12.0, row_y + WAVEFORM_ITEM_HEIGHT / 2.0),
            color: TEXT_COLOR,
            size: 13.0 * zoom,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
    }
}

/// A white outline just outside the selected node.
pub(super) fn draw_selection(frame: &mut Frame, node: &FlodeNode) {
    let bounds = node.bounds();
    let outline = Rectangle {
        x: bounds.x - SELECTION_WIDTH,
        y: bounds.y - SELECTION_WIDTH,
        width: bounds.width + 2.0 * SELECTION_WIDTH,
        height: bounds.height + 2.0 * SELECTION_WIDTH,
    };
    frame.stroke(
        &rounded_rect(outline, CORNER_RADIUS + SELECTION_WIDTH),
        Stroke::default()
            .with_color(SELECTION_COLOR)
            .with_width(SELECTION_WIDTH),
    );
}

/// The pill of add buttons at the top of the editor.
pub(super) fn draw_pill(frame: &mut Frame, bounds: Rectangle) {
    let pill = pill_bounds(bounds);
    frame.fill(&rounded_rect(pill, PILL_HEIGHT / 2.0), PILL_COLOR);

    let segment = pill.width / PILL_ITEMS.len() as f32;
    for divider in 1..PILL_ITEMS.len() {
        let x = pill.x + divider as f32 * segment;
        frame.stroke(
            &Path::line(
                Point::new(x, pill.y + 8.0),
                Point::new(x, pill.y + pill.height - 8.0),
            ),
            Stroke::default()
                .with_color(PILL_DIVIDER_COLOR)
                .with_width(1.0),
        );
    }

    for (index, item) in PILL_ITEMS.iter().enumerate() {
        let center_x = pill.x + (index as f32 + 0.5) * segment;
        let center_y = pill.y + pill.height / 2.0;
        let label = match item {
            PillItem::Kind(kind) => kind.title(),
            PillItem::Category(category) => category.label(),
        };
        frame.fill_text(Text {
            content: label.to_string(),
            position: Point::new(center_x, center_y),
            color: TEXT_COLOR,
            size: 14.0,
            horizontal_alignment: alignment::Horizontal::Center,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
        // A category opens a list, so its button has a chevron.
        if let PillItem::Category(_) = item {
            let chevron_x = center_x + 36.0;
            let chevron = Path::new(|builder| {
                builder.move_to(Point::new(chevron_x - 3.0, center_y - 2.0));
                builder.line_to(Point::new(chevron_x, center_y + 2.0));
                builder.line_to(Point::new(chevron_x + 3.0, center_y - 2.0));
            });
            frame.stroke(
                &chevron,
                Stroke::default()
                    .with_color(TEXT_COLOR)
                    .with_width(1.5)
                    .with_line_cap(LineCap::Round),
            );
        }
    }
}

/// The list under an open category. Its rows are the category's node kinds, or a note when the
/// category has none yet.
pub(super) fn draw_category_list(frame: &mut Frame, bounds: Rectangle, category: Category) {
    let list = category_list_bounds(bounds, category);
    frame.fill(&rounded_rect(list, MENU_CORNER_RADIUS), MENU_COLOR);

    let labels: Vec<&str> = if category.kinds().is_empty() {
        vec!["Empty for now"]
    } else {
        category.kinds().iter().map(|kind| kind.title()).collect()
    };
    for (row, label) in labels.iter().enumerate() {
        frame.fill_text(Text {
            content: label.to_string(),
            position: Point::new(
                list.x + 12.0,
                list.y + row as f32 * MENU_ITEM_HEIGHT + MENU_ITEM_HEIGHT / 2.0,
            ),
            color: if category.kinds().is_empty() {
                DIM_TEXT_COLOR
            } else {
                TEXT_COLOR
            },
            size: 13.0,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
    }
}

/// The right-click menu: one row for each entry the node's kind offers.
pub(super) fn draw_menu(frame: &mut Frame, menu: ContextMenu, node: &FlodeNode, zoom: f32) {
    let bounds = menu_bounds(menu);
    frame.fill(&rounded_rect(bounds, MENU_CORNER_RADIUS), MENU_COLOR);
    for (row, item) in menu.items.iter().enumerate() {
        frame.fill_text(Text {
            content: item.label(node).to_string(),
            position: Point::new(
                bounds.x + 12.0,
                bounds.y + row as f32 * MENU_ITEM_HEIGHT + MENU_ITEM_HEIGHT / 2.0,
            ),
            color: TEXT_COLOR,
            size: 14.0 * zoom,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
    }
}

/// A rectangle with rounded corners of radius `r`. Each corner is a quadratic curve whose control
/// point is the sharp corner. Iced's arc builders produced skewed corners here.
pub(super) fn rounded_rect(bounds: Rectangle, r: f32) -> Path {
    let x0 = bounds.x;
    let y0 = bounds.y;
    let x1 = bounds.x + bounds.width;
    let y1 = bounds.y + bounds.height;
    Path::new(|builder| {
        builder.move_to(Point::new(x0 + r, y0));
        builder.line_to(Point::new(x1 - r, y0));
        builder.quadratic_curve_to(Point::new(x1, y0), Point::new(x1, y0 + r));
        builder.line_to(Point::new(x1, y1 - r));
        builder.quadratic_curve_to(Point::new(x1, y1), Point::new(x1 - r, y1));
        builder.line_to(Point::new(x0 + r, y1));
        builder.quadratic_curve_to(Point::new(x0, y1), Point::new(x0, y1 - r));
        builder.line_to(Point::new(x0, y0 + r));
        builder.quadratic_curve_to(Point::new(x0, y0), Point::new(x0 + r, y0));
        builder.close();
    })
}
