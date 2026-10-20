//! Everything the editor paints: wires, nodes, knobs, dropdowns, the pill, and the lists.

use crate::dsp::analyzer::{BINS, FLOOR_DB};
use crate::editor::knob::{self};
use crate::editor::piano::{self};
use crate::nodes::eq;
use crate::nodes::knobs::KnobId;
use crate::nodes::map;
use crate::nodes::viewer::Live;
use crate::nodes::{
    Category, FlodeNode, NodeKind, Port, RowShape, Signal, ARROW_CENTER_X, HEADER_HEIGHT,
    ROW_KNOB_DIAMETER, SECTION_HEIGHT, TITLE_INSET,
};
use crate::patch::control::ControlLevels;
use nih_plug_iced::widget::canvas::{Frame, Geometry, LineCap, Path, Stroke, Text};
use nih_plug_iced::{alignment, Color, Point, Rectangle};

use super::depth::{self, draw_shadow, FLOAT_SHADOW, LIFTED_SHADOW, NODE_SHADOW};
use super::{curve::*, graph::*, logo, menus::*, style::*};

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
    // The logo sits over the canvas too, so node text under it is hidden as well. It goes right
    // after the pill and strip, and `fixed` counts the overlays that are not open menus.
    let logo = logo::logo_bounds(bounds);
    let fixed = overlays.len() + usize::from(logo.is_some());
    if let Some(rect) = logo {
        overlays.push(view.rect_to_world(rect));
    }
    if let Some((index, which)) = graph.dropdown_menu {
        let node = &graph.nodes[index];
        if let Some(dropdown) = node.dropdowns().into_iter().find(|d| d.which == which) {
            // The pin labels sit left of the list, so the cover spans the node's full width.
            let list = dropdown_list_bounds(dropdown.bounds, dropdown.labels.len());
            overlays.push(Rectangle {
                x: node.position.x,
                y: list.y,
                width: node.width(),
                height: list.height,
            });
        }
    }
    if let Some(menu) = graph.menu {
        overlays.push(view.rect_to_world(menu_bounds(menu)));
    }
    if let Some(category) = graph.category_menu {
        overlays.push(view.rect_to_world(category_list_bounds(bounds, category)));
    }
    if let Some(menu) = &graph.preset_list {
        overlays.push(view.rect_to_world(preset_list_bounds(bounds, &menu.library)));
        if let Some(open) = menu.open {
            overlays.push(view.rect_to_world(submenu_bounds(bounds, &menu.library, open)));
        }
    }
    if let Some(open) = &graph.quick_add {
        let rows = graph.quick_add_results().len();
        overlays.push(view.rect_to_world(quick_add_bounds(bounds, open.screen, rows)));
    }
    // The number box covers the label text of the node it sits on, which is in editor coordinates.
    if let Some(rect) = graph.knob_entry_bounds() {
        overlays.push(rect);
    }

    // The nodes and wires are drawn in editor coordinates, then the view scales and moves them.
    frame.with_save(|frame| {
        frame.translate(offset);
        frame.scale(view.zoom);

        // The dot grid is behind everything. It covers the part of the editor in view.
        let screen = Rectangle {
            x: 0.0,
            y: 0.0,
            width: bounds.width,
            height: bounds.height,
        };
        depth::draw_grid(frame, view.rect_to_world(screen), view.zoom);

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
                let color = match out.signal() {
                    Signal::Audio => PENDING_WIRE_COLOR,
                    Signal::Control => CONTROL_COLOR,
                    Signal::Notes => NOTES_COLOR,
                };
                draw_curve(frame, &wire_curve(start, cursor), color, 2.0);
            }
        }

        // A wire dragged back from an input runs from the cursor to the input.
        if let Interaction::BackWiring { to, input, cursor } = graph.interaction {
            if let Some(end) = graph.nodes[to].input_point(input) {
                let color = match input.signal() {
                    Signal::Audio => PENDING_WIRE_COLOR,
                    Signal::Control => CONTROL_COLOR,
                    Signal::Notes => NOTES_COLOR,
                };
                draw_curve(frame, &wire_curve(cursor, end), color, 2.0);
            }
        }

        for index in graph.selection() {
            draw_selection(frame, &graph.nodes[index]);
        }

        if let (Some(entry), Some(rect)) = (&graph.knob_entry, graph.knob_entry_bounds()) {
            draw_knob_entry(frame, entry, rect, view.zoom);
        }

        if let Interaction::AutoLinking {
            from,
            start,
            cursor,
        } = graph.interaction
        {
            // The node the drag would connect to is outlined, and the line runs to the cursor.
            if let Some(target) = graph.node_at(cursor).filter(|&node| node != from) {
                draw_selection(frame, &graph.nodes[target]);
            }
            frame.stroke(
                &Path::line(start, cursor),
                Stroke::default()
                    .with_color(AUTO_LINK_COLOR)
                    .with_width(3.0)
                    .with_line_cap(LineCap::Round),
            );
            frame.fill(&Path::circle(start, 6.0), AUTO_LINK_COLOR);
        }

        if let Interaction::Boxing { start, cursor } = graph.interaction {
            draw_selection_box(frame, start, cursor);
        }
    });

    // The pill stays fixed on screen. It is drawn after the nodes, so it covers them. Its buttons
    // light when the cursor is over them, and while their list is open.
    let hovered_item = if graph.cursor_in {
        pill_button_at(bounds, graph.last_screen)
    } else {
        None
    };
    let item_place = |item: PillItem| PILL_ITEMS.iter().position(|candidate| *candidate == item);
    let open_item = match (graph.category_menu, &graph.preset_list) {
        (Some(category), _) => item_place(PillItem::Category(category)),
        (None, Some(_)) => item_place(PillItem::Presets),
        (None, None) => None,
    };
    draw_pill(
        &mut frame,
        bounds,
        graph.preset_name.as_deref(),
        hovered_item.and_then(item_place),
        open_item,
    );
    if let Some(rect) = logo {
        logo::draw_logo(&mut frame, rect);
    }
    draw_status(&mut frame, bounds, graph);
    super::meter::draw(&mut frame, bounds, output, graph.midi_light);

    let strip = piano::strip_bounds(bounds);
    // Text would show through a menu drawn over the strip, so the strip drops its labels then.
    let strip_world = overlays[1];
    let labels = !overlays[fixed..]
        .iter()
        .any(|overlay| overlaps(overlay, &strip_world));
    piano::draw_strip(
        &mut frame,
        strip,
        &piano::StripView {
            octave: graph.piano.octave(),
            sounding: active_notes,
            pressed: graph.mouse_note,
            computer_keys: graph.computer_keys,
            hover: if graph.cursor_in {
                piano::octave_button_at(strip, graph.last_screen)
            } else {
                None
            },
            labels,
        },
    );

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
                let disabled: Vec<bool> = (0..dropdown.labels.len())
                    .map(|row| graph.dropdown_row_disabled(index, which, row))
                    .collect();
                draw_list(
                    frame,
                    list,
                    &dropdown.labels,
                    dropdown.selected,
                    hovered_row,
                    &disabled,
                    view.zoom,
                );
            }
        }
    });

    // The right-click menu is in screen coordinates, so it keeps one size at any zoom.
    if let Some(menu) = graph.menu {
        if let Some(node) = graph.nodes.get(menu.node) {
            let hovered_row = if graph.cursor_in {
                menu_item_at(menu, graph.last_screen)
                    .and_then(|item| menu.items.iter().position(|&i| i == item))
            } else {
                None
            };
            draw_menu(&mut frame, menu, node, hovered_row);
        }
    }

    if let Some(category) = graph.category_menu {
        draw_category_list(&mut frame, bounds, category, graph);
    }
    draw_preset_list(&mut frame, bounds, graph);
    if graph.quick_add.is_some() {
        draw_quick_add(&mut frame, bounds, graph);
    }

    vec![frame.into_geometry()]
}

/// Audio is grey, control is green, and notes are purple.
pub(super) fn wire_color(out: Port) -> Color {
    match out.signal() {
        Signal::Audio => WIRE_COLOR,
        Signal::Control => CONTROL_COLOR,
        Signal::Notes => NOTES_COLOR,
    }
}

/// The four points of a wire's cubic curve. Control points stay out from the pins so a wire
/// leaves and enters level, up to 60 pixels, but never farther than half the wire's length. A
/// longer reach on a short wire makes the handles cross and kink the curve.
pub(super) fn wire_curve(from: Point, to: Point) -> [Point; 4] {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let length = dx.hypot(dy);
    let reach = (dx.abs() * 0.5).max((length * 0.5).min(MIN_WIRE_REACH));
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
    // A faint, wider copy of the wire's own color under it gives it a soft halo.
    for (extra, alpha) in WIRE_GLOW {
        frame.stroke(
            &path,
            Stroke::default()
                .with_color(Color { a: alpha, ..color })
                .with_width(width + extra)
                .with_line_cap(LineCap::Round),
        );
    }
    frame.stroke(
        &path,
        Stroke::default()
            .with_color(color)
            .with_width(width)
            .with_line_cap(LineCap::Round),
    );
}

/// The halo around a wire: how much wider than the wire each layer is, and its opacity. The
/// widest layer is the faintest.
const WIRE_GLOW: [(f32, f32); 2] = [(6.0, 0.04), (3.0, 0.08)];

/// The header tag of an LFO or envelope that is past the limit and so makes no signal.
pub(super) fn over_limit_label(graph: &Graph, index: usize) -> Option<&'static str> {
    let wiring = graph.wiring();
    let running = match graph.nodes[index].kind {
        NodeKind::Lfo => wiring.lfo_index(index).is_some(),
        NodeKind::Envelope => wiring.envelope_index(index).is_some(),
        _ => true,
    };
    (!running).then_some("Over limit")
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
    // A muted or bypassed node is darker, and says which in its header. So is an LFO or envelope
    // past the ones the audio thread runs, since it makes no signal.
    let off = node
        .bypass_label()
        .or_else(|| over_limit_label(graph, index));
    let (body_color, header_color) = if off.is_some() {
        (NODE_OFF_BODY_COLOR, NODE_OFF_HEADER_COLOR)
    } else {
        (NODE_BODY_COLOR, NODE_HEADER_COLOR)
    };
    // The selected node sits higher, so its shadow is bigger.
    let shadow = if graph.is_selected(index) {
        LIFTED_SHADOW
    } else {
        NODE_SHADOW
    };
    draw_shadow(frame, bounds, CORNER_RADIUS, shadow);
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

    // A faint edge all round, and a brighter line along the top of the header, so the node
    // catches a little light.
    frame.stroke(
        &rounded_rect(bounds, CORNER_RADIUS),
        Stroke::default()
            .with_color(Color {
                a: 0.06,
                ..Color::WHITE
            })
            .with_width(1.0),
    );
    frame.stroke(
        &Path::line(
            Point::new(header.x + CORNER_RADIUS, header.y + 0.5),
            Point::new(header.x + header.width - CORNER_RADIUS, header.y + 0.5),
        ),
        Stroke::default()
            .with_color(Color {
                a: 0.12,
                ..Color::WHITE
            })
            .with_width(1.0),
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

    // Each row has an optional input pin on the left edge, a knob with its name and value, and an
    // optional output pin on the right edge. A row in a folded section, or in a collapsed node,
    // keeps only its pins, packed close together, and wires still attach to them.
    let places = node.row_places();
    for (row_index, row) in node.rows().iter().enumerate() {
        let place = places[row_index];
        let y = node.position.y + place.y;
        let left = node.position.x;
        let right = node.position.x + node.width();

        match place.shape {
            RowShape::Hidden => continue,
            RowShape::SectionHeader(n) => {
                draw_section_header(frame, node, n, y, covered, zoom);
                continue;
            }
            RowShape::Packed => {
                if let Some(port) = row.input {
                    draw_packed_pin_dot(frame, Point::new(left, y), port);
                }
                if let Some(port) = row.output {
                    draw_packed_pin_dot(frame, Point::new(right, y), port);
                }
                continue;
            }
            RowShape::Full => {}
        }

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
            // A pin with no knob to name it draws its own label, such as "Shape" or "A". A pin on
            // a dropdown's row has no room for one.
            if row.knob.is_none() && row.control.is_none() {
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

    // A collapsed node is its header and its pins, and nothing below them.
    if node.collapsed {
        return;
    }

    {
        let labels = node.switch_labels();
        for (index, label) in labels.iter().enumerate() {
            if node.switch_shown(index) {
                draw_switch(frame, node, index, label, covered, zoom);
            }
        }
    }

    for dropdown in node.dropdowns() {
        let hovered = graph.hover
            == Some(Hover::Button {
                node: index,
                which: dropdown.which,
            });
        // A chosen value that can't work right now, such as FM on a mix that runs on the bus,
        // shows dimmed.
        let dim = graph.dropdown_row_disabled(index, dropdown.which, dropdown.selected);
        draw_dropdown(
            frame,
            dropdown.bounds,
            dropdown.name,
            dropdown.labels[dropdown.selected],
            hovered,
            dim,
            zoom,
            covered,
        );
    }

    if node.has_curve() {
        draw_map_curve(frame, index, graph, levels);
    }

    if node.has_viewer() {
        draw_viewer(frame, index, graph, levels);
    }

    if node.eq_bounds().is_some() {
        draw_eq(frame, index, graph, levels, covered, zoom);
    }

    if node.monitor_bounds().is_some() {
        super::monitor::draw_monitor(frame, index, graph, covered, zoom);
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

/// The header of a section: a band across the node with an arrow and the section's title. The
/// arrow points down when the section is open and right when it is folded.
pub(super) fn draw_section_header(
    frame: &mut Frame,
    node: &FlodeNode,
    section: usize,
    y: f32,
    covered: &[Rectangle],
    zoom: f32,
) {
    let left = node.position.x;
    let band = Rectangle {
        x: left + 6.0,
        y: y - SECTION_HEIGHT / 2.0 + 2.0,
        width: node.width() - 12.0,
        height: SECTION_HEIGHT - 4.0,
    };
    frame.fill(&rounded_rect(band, DROPDOWN_CORNER_RADIUS), SECTION_COLOR);

    let folded = node.section_collapsed(section);
    let x = left + SECTION_ARROW_X;
    let arrow = if folded {
        Path::new(|builder| {
            builder.move_to(Point::new(x - 2.0, y - 3.5));
            builder.line_to(Point::new(x + 2.5, y));
            builder.line_to(Point::new(x - 2.0, y + 3.5));
            builder.close();
        })
    } else {
        Path::new(|builder| {
            builder.move_to(Point::new(x - 3.5, y - 2.0));
            builder.line_to(Point::new(x + 3.5, y - 2.0));
            builder.line_to(Point::new(x, y + 2.5));
            builder.close();
        })
    };
    frame.fill(&arrow, SECTION_TEXT_COLOR);

    let title = node
        .kind
        .behavior()
        .sections()
        .get(section)
        .map_or("", |section| section.title);
    draw_text(
        frame,
        covered,
        zoom,
        Text {
            content: title.to_string(),
            position: Point::new(left + SECTION_ARROW_X + 12.0, y),
            color: SECTION_TEXT_COLOR,
            size: 12.0,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        },
    );
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

/// A pin without its label. Audio pins are grey and big, control pins are green, and notes pins
/// are purple. The last two are the smaller size.
pub(super) fn draw_pin_dot(frame: &mut Frame, point: Point, port: Port) {
    match port.signal() {
        Signal::Audio => frame.fill(&Path::circle(point, PIN_RADIUS), TERMINAL_COLOR),
        Signal::Control => frame.fill(&Path::circle(point, CONTROL_PIN_RADIUS), CONTROL_COLOR),
        Signal::Notes => frame.fill(&Path::circle(point, CONTROL_PIN_RADIUS), NOTES_COLOR),
    }
}

/// A pin of a folded section or node: the same colors as an ordinary pin, but smaller, so a tight
/// stack of them does not run together.
pub(super) fn draw_packed_pin_dot(frame: &mut Frame, point: Point, port: Port) {
    match port.signal() {
        Signal::Audio => frame.fill(
            &Path::circle(point, PACKED_PIN_RADIUS + 0.5),
            TERMINAL_COLOR,
        ),
        Signal::Control => frame.fill(&Path::circle(point, PACKED_PIN_RADIUS), CONTROL_COLOR),
        Signal::Notes => frame.fill(&Path::circle(point, PACKED_PIN_RADIUS), NOTES_COLOR),
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

    // The curve reads the live values, so a value a wire drives moves it and its handle.
    let live = live_knobs(graph, index, &levels);
    let stage = map::live_stage(node, &live);
    // A bipolar map is a center and a range, so a faint line marks the center of the swing.
    if map::bipolar(node) {
        let (low, high) = map::output_ends(node, &live);
        let center = ((low + high) / 2.0 + map::offset(node, &live)).clamp(0.0, 1.0);
        let y = bounds.y + (1.0 - center) * bounds.height;
        frame.stroke(
            &Path::line(
                Point::new(bounds.x, y),
                Point::new(bounds.x + bounds.width, y),
            ),
            Stroke::default()
                .with_color(Color {
                    a: 0.2,
                    ..TEXT_COLOR
                })
                .with_width(1.0),
        );
    }
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

    for handle in map_handles(node, &live) {
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

/// The knob values a node's viewer reads: the node's own, except for a knob a control wire drives,
/// which reads that wire's live level. This is what makes the shape move with a knob or its wire.
pub(super) fn live_knobs(graph: &Graph, index: usize, levels: &ControlLevels) -> Live {
    let node = &graph.nodes[index];
    let pinned: Vec<KnobId> = node
        .rows()
        .iter()
        .filter_map(|row| match row.input {
            Some(Port::Knob(id)) => Some(id),
            _ => None,
        })
        .collect();
    let wiring = graph.wiring();
    let mut live = Live::new(node, |id| {
        if pinned.contains(&id) {
            wiring
                .control_route(index, id)
                .map(|route| route.apply(levels))
        } else {
            None
        }
    });
    // An envelope and an LFO also say where their signal is, for the viewer's cursor.
    match node.kind {
        NodeKind::Envelope => {
            if let Some(slot) = wiring.envelope_index(index) {
                if let Some(phase) = levels.envelope_phase[slot] {
                    live = live.with_envelope(phase, levels.envelope[slot]);
                }
            }
        }
        NodeKind::Lfo => {
            if let Some(slot) = wiring.lfo_index(index) {
                live = live.with_lfo(levels.lfo_phase[slot], levels.lfo[slot]);
            }
        }
        _ => {}
    }
    live
}

/// A node's viewer: a box with the shape the node's kind traces in it. Only the handles of an
/// envelope and of a custom LFO curve are clickable.
pub(super) fn draw_viewer(frame: &mut Frame, index: usize, graph: &Graph, levels: ControlLevels) {
    let node = &graph.nodes[index];
    let Some(bounds) = node.viewer_bounds() else {
        return;
    };
    frame.fill(
        &rounded_rect(bounds, DROPDOWN_CORNER_RADIUS),
        DROPDOWN_COLOR,
    );

    // A faint midline, so a shape around zero reads as one.
    let mid = bounds.y + bounds.height / 2.0;
    frame.stroke(
        &Path::line(
            Point::new(bounds.x, mid),
            Point::new(bounds.x + bounds.width, mid),
        ),
        Stroke::default()
            .with_color(Color {
                a: 0.12,
                ..TEXT_COLOR
            })
            .with_width(1.0),
    );

    let live = live_knobs(graph, index, &levels);
    let Some(trace) = node.trace(&live) else {
        return;
    };
    let at = |i: usize, y: f32| {
        let x = i as f32 / trace.points.len().saturating_sub(1).max(1) as f32;
        Point::new(
            bounds.x + x * bounds.width,
            bounds.y + (1.0 - y) * bounds.height,
        )
    };

    for marker in &trace.markers {
        let x = bounds.x + marker.clamp(0.0, 1.0) * bounds.width;
        frame.stroke(
            &Path::line(
                Point::new(x, bounds.y),
                Point::new(x, bounds.y + bounds.height),
            ),
            Stroke::default()
                .with_color(Color {
                    a: 0.25,
                    ..TEXT_COLOR
                })
                .with_width(1.0),
        );
    }

    if trace.points.len() < 2 {
        return;
    }
    if trace.fill {
        let area = Path::new(|builder| {
            builder.move_to(Point::new(bounds.x, bounds.y + bounds.height));
            for (i, y) in trace.points.iter().enumerate() {
                builder.line_to(at(i, *y));
            }
            builder.line_to(Point::new(
                bounds.x + bounds.width,
                bounds.y + bounds.height,
            ));
            builder.close();
        });
        frame.fill(
            &area,
            Color {
                a: 0.15,
                ..TEXT_COLOR
            },
        );
    }
    let line = Path::new(|builder| {
        for (i, y) in trace.points.iter().enumerate() {
            if i == 0 {
                builder.move_to(at(i, *y));
            } else {
                builder.line_to(at(i, *y));
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

    // An envelope's curve has a handle at the end of each stage, drawn like the EQ's points.
    if let Some(handles) = envelope_handles(node, &live) {
        for (handle, center) in handles.iter().enumerate() {
            let held = graph.interaction
                == Interaction::EnvelopeHandle {
                    node: index,
                    handle,
                };
            frame.fill(
                &Path::circle(*center, if held { 5.5 } else { 4.5 }),
                TEXT_COLOR,
            );
        }
    }

    // An LFO on Custom has a handle at each point of its curve, in the same style.
    if let Some(handles) = lfo_handles(node) {
        for (point, center) in handles.iter().enumerate() {
            let held = graph.interaction == Interaction::LfoPoint { node: index, point };
            frame.fill(
                &Path::circle(*center, if held { 5.5 } else { 4.5 }),
                TEXT_COLOR,
            );
        }
    }

    // The live signal, as the dot the map's curve also uses.
    if let Some((x, y)) = trace.cursor {
        let dot = Point::new(
            bounds.x + x.clamp(0.0, 1.0) * bounds.width,
            bounds.y + (1.0 - y.clamp(0.0, 1.0)) * bounds.height,
        );
        frame.fill(&Path::circle(dot, 3.5), piano::SOUNDING_DOT_COLOR);
    }
}

/// The cubic Bézier segments of a smooth curve through `points`, as (first control, second control,
/// end) for each step after the first point. It is a Catmull-Rom spline, so the curve passes
/// through every point with no kink. A control point stays between `top` and `bottom`, so a sharp
/// peak does not swing the curve out of the box.
pub(super) fn smooth_segments(
    points: &[Point],
    top: f32,
    bottom: f32,
) -> Vec<(Point, Point, Point)> {
    let at = |i: isize| points[i.clamp(0, points.len() as isize - 1) as usize];
    let keep = |p: Point| Point::new(p.x, p.y.clamp(top, bottom));
    (0..points.len().saturating_sub(1) as isize)
        .map(|i| {
            let (before, start, end, after) = (at(i - 1), at(i), at(i + 1), at(i + 2));
            let c1 = Point::new(
                start.x + (end.x - before.x) / 6.0,
                start.y + (end.y - before.y) / 6.0,
            );
            let c2 = Point::new(
                end.x - (after.x - start.x) / 6.0,
                end.y - (after.y - start.y) / 6.0,
            );
            (keep(c1), keep(c2), end)
        })
        .collect()
}

/// The EQ node's graph: a grid, the combined response of the four bands, and a numbered point for
/// each band. The numbers match the Q knobs. Each band's toggle is drawn beside its Q knob.
pub(super) fn draw_eq(
    frame: &mut Frame,
    index: usize,
    graph: &Graph,
    levels: ControlLevels,
    covered: &[Rectangle],
    zoom: f32,
) {
    let node = &graph.nodes[index];
    let live = live_knobs(graph, index, &levels);
    let (Some(bounds), Some(handles)) = (node.eq_bounds(), eq_handles(node, &live)) else {
        return;
    };
    frame.fill(
        &rounded_rect(bounds, DROPDOWN_CORNER_RADIUS),
        DROPDOWN_COLOR,
    );

    let faint = |alpha: f32| {
        Stroke::default()
            .with_color(Color {
                a: alpha,
                ..TEXT_COLOR
            })
            .with_width(1.0)
    };
    // Lines at each decade of frequency, and a brighter one at 0 dB.
    for freq in [100.0, 1000.0, 10000.0] {
        let x = bounds.x + eq::position_of(freq) * bounds.width;
        frame.stroke(
            &Path::line(
                Point::new(x, bounds.y),
                Point::new(x, bounds.y + bounds.height),
            ),
            faint(0.08),
        );
    }
    let mid = bounds.y + bounds.height / 2.0;
    frame.stroke(
        &Path::line(
            Point::new(bounds.x, mid),
            Point::new(bounds.x + bounds.width, mid),
        ),
        faint(0.2),
    );

    // The analyzer's spectrum sits behind the curve, as a filled shape on the same axes. Each band
    // is one point, at its middle on the log axis, and its level maps from the floor to 0 dB.
    if let Some(bins) = graph.eq_spectrum(index) {
        let at = |i: usize, db: f32| {
            let x = (i as f32 + 0.5) / BINS as f32;
            let y = ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0);
            Point::new(
                bounds.x + x * bounds.width,
                bounds.y + (1.0 - y) * bounds.height,
            )
        };
        let bottom = bounds.y + bounds.height;
        let points: Vec<Point> = bins.iter().enumerate().map(|(i, db)| at(i, *db)).collect();
        let area = Path::new(|builder| {
            builder.move_to(Point::new(points[0].x, bottom));
            builder.line_to(points[0]);
            // A smooth curve through the points, so the bands do not read as straight facets.
            for (c1, c2, end) in smooth_segments(&points, bounds.y, bottom) {
                builder.bezier_curve_to(c1, c2, end);
            }
            builder.line_to(Point::new(points[BINS - 1].x, bottom));
            builder.close();
        });
        frame.fill(&area, ANALYZER_COLOR);
    }

    // The response reads Q through the live values, so a wire that drives a Q moves the curve.
    let params = eq::params(node, &live);
    let samples = bounds.width as usize;
    let curve = Path::new(|builder| {
        for i in 0..=samples {
            let x = i as f32 / samples as f32;
            let db = crate::dsp::effects::eq::response_db(
                &params,
                eq::frequency_at(x),
                eq::VIEW_SAMPLE_RATE,
            );
            let point = Point::new(
                bounds.x + x * bounds.width,
                bounds.y + (1.0 - eq::height_of(db)) * bounds.height,
            );
            if i == 0 {
                builder.move_to(point);
            } else {
                builder.line_to(point);
            }
        }
    });
    frame.stroke(
        &curve,
        Stroke::default()
            .with_color(TEXT_COLOR)
            .with_width(1.5)
            .with_line_cap(LineCap::Round),
    );

    for (band, handle) in handles.iter().enumerate() {
        let held = graph.interaction == Interaction::EqPoint { node: index, band };
        frame.fill(
            &Path::circle(*handle, if held { 5.5 } else { 4.5 }),
            TEXT_COLOR,
        );
        draw_text(
            frame,
            covered,
            zoom,
            Text {
                content: (band + 1).to_string(),
                position: Point::new(handle.x, (handle.y - 11.0).max(bounds.y + 6.0)),
                color: DIM_TEXT_COLOR,
                size: 10.0,
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Center,
                ..Text::default()
            },
        );
    }

    // Each toggle is a button with a small picture of what its band does now.
    for (band, toggle) in node.eq_toggles() {
        frame.fill(
            &rounded_rect(toggle, DROPDOWN_CORNER_RADIUS),
            DROPDOWN_COLOR,
        );
        let art = Rectangle {
            x: toggle.x + 7.0,
            y: toggle.y + 5.0,
            width: toggle.width - 14.0,
            height: toggle.height - 10.0,
        };
        let icon = Path::new(|builder| {
            for (i, (x, y)) in eq::toggle_icon(node, band).into_iter().enumerate() {
                let point = Point::new(art.x + x * art.width, art.y + y * art.height);
                if i == 0 {
                    builder.move_to(point);
                } else {
                    builder.line_to(point);
                }
            }
        });
        frame.stroke(
            &icon,
            Stroke::default()
                .with_color(TEXT_COLOR)
                .with_width(1.5)
                .with_line_cap(LineCap::Round),
        );
    }
}

/// The box for typing a knob's value: the text with a caret, over the knob. The text is on a
/// lighter ground while it is still selected, so it reads as the thing the next key replaces.
pub(super) fn draw_knob_entry(frame: &mut Frame, entry: &KnobEntry, rect: Rectangle, zoom: f32) {
    draw_shadow(frame, rect, DROPDOWN_CORNER_RADIUS, FLOAT_SHADOW);
    frame.fill(&rounded_rect(rect, DROPDOWN_CORNER_RADIUS), DROPDOWN_COLOR);
    frame.stroke(
        &rounded_rect(rect, DROPDOWN_CORNER_RADIUS),
        Stroke::default()
            .with_color(SELECTION_COLOR)
            .with_width(1.0),
    );
    let content = if entry.fresh {
        entry.text.clone()
    } else {
        format!("{}|", entry.text)
    };
    if entry.fresh {
        // A selection band behind the text.
        let width = (entry.text.chars().count() as f32 * 7.0 + 8.0).min(rect.width - 8.0);
        let band = Rectangle {
            x: rect.x + (rect.width - width) / 2.0,
            y: rect.y + 3.0,
            width,
            height: rect.height - 6.0,
        };
        frame.fill(
            &rounded_rect(band, 3.0),
            Color {
                a: 0.45,
                ..SELECTION_COLOR
            },
        );
    }
    draw_text(
        frame,
        &[],
        zoom,
        Text {
            content,
            position: Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0),
            color: TEXT_COLOR,
            size: 13.0,
            horizontal_alignment: alignment::Horizontal::Center,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        },
    );
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

/// The closed dropdown: the current label and a chevron. `dim` draws the label dimmed, for a value
/// that can't take effect.
pub(super) fn draw_dropdown(
    frame: &mut Frame,
    bounds: Rectangle,
    name: &str,
    label: &str,
    hovered: bool,
    dim: bool,
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
            color: if dim { DIM_TEXT_COLOR } else { TEXT_COLOR },
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

/// The open dropdown list. The selected row is highlighted. A row `disabled` marks is dimmed.
pub(super) fn draw_list(
    frame: &mut Frame,
    list: Rectangle,
    labels: &[&str],
    selected: usize,
    hovered_row: Option<usize>,
    disabled: &[bool],
    zoom: f32,
) {
    draw_shadow(frame, list, MENU_CORNER_RADIUS, FLOAT_SHADOW);
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
            color: if disabled.get(row).copied().unwrap_or(false) {
                DIM_TEXT_COLOR
            } else {
                TEXT_COLOR
            },
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
    // A soft glow in the accent color, widest and faintest first, then the outline itself.
    let outline_path = rounded_rect(outline, CORNER_RADIUS + SELECTION_WIDTH);
    for (extra, alpha) in SELECTION_GLOW {
        frame.stroke(
            &outline_path,
            Stroke::default()
                .with_color(Color {
                    a: alpha,
                    ..SELECTION_COLOR
                })
                .with_width(SELECTION_WIDTH + extra),
        );
    }
    frame.stroke(
        &outline_path,
        Stroke::default()
            .with_color(SELECTION_COLOR)
            .with_width(SELECTION_WIDTH),
    );
}

/// The box a drag over empty canvas selects with: a faint fill and a thin outline in the accent
/// color.
fn draw_selection_box(frame: &mut Frame, a: Point, b: Point) {
    let area = Rectangle {
        x: a.x.min(b.x),
        y: a.y.min(b.y),
        width: (a.x - b.x).abs(),
        height: (a.y - b.y).abs(),
    };
    let path = Path::rectangle(
        Point::new(area.x, area.y),
        nih_plug_iced::Size::new(area.width, area.height),
    );
    frame.fill(
        &path,
        Color {
            a: 0.08,
            ..SELECTION_COLOR
        },
    );
    frame.stroke(
        &path,
        Stroke::default()
            .with_color(Color {
                a: 0.6,
                ..SELECTION_COLOR
            })
            .with_width(1.0),
    );
}

/// The glow around the selected node: how much wider than the outline each layer is, and its
/// opacity.
const SELECTION_GLOW: [(f32, f32); 3] = [(9.0, 0.04), (5.5, 0.07), (2.5, 0.12)];

/// The pill of add buttons at the top of the editor.
///
/// `hovered` is the button under the cursor and `open` is the one whose list is open, each by its
/// place in `PILL_ITEMS`. Both are lit, and their chevron goes from dim to white.
pub(super) fn draw_pill(
    frame: &mut Frame,
    bounds: Rectangle,
    preset_name: Option<&str>,
    hovered: Option<usize>,
    open: Option<usize>,
) {
    let pill = pill_bounds(bounds);
    draw_shadow(frame, pill, PILL_HEIGHT / 2.0, FLOAT_SHADOW);
    frame.fill(&rounded_rect(pill, PILL_HEIGHT / 2.0), PILL_COLOR);

    let segment = pill.width / PILL_ITEMS.len() as f32;
    for (index, color) in [
        (open, DROPDOWN_HOVER_COLOR),
        (hovered, LIST_ROW_HOVER_COLOR),
    ]
    .into_iter()
    .filter_map(|(item, color)| item.map(|item| (item, color)))
    {
        let lit = Rectangle {
            x: pill.x + index as f32 * segment + PILL_LIGHT_INSET,
            y: pill.y + PILL_LIGHT_INSET,
            width: segment - 2.0 * PILL_LIGHT_INSET,
            height: pill.height - 2.0 * PILL_LIGHT_INSET,
        };
        frame.fill(&rounded_rect(lit, lit.height / 2.0), color);
    }
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
            PillItem::Kind(kind) => kind.title().to_string(),
            PillItem::Category(category) => category.label().to_string(),
            // The button names the current preset, cut short to fit.
            PillItem::Presets => preset_name.map_or("Presets".to_string(), shorten),
        };
        frame.fill_text(Text {
            content: label,
            position: Point::new(center_x, center_y),
            color: TEXT_COLOR,
            size: 14.0,
            horizontal_alignment: alignment::Horizontal::Center,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
        // A category and the presets open a list, so their buttons have a chevron.
        if let PillItem::Category(_) | PillItem::Presets = item {
            // The Presets button can hold a name, so its chevron sits further out.
            let chevron_x = center_x
                + if *item == PillItem::Presets {
                    50.0
                } else {
                    36.0
                };
            let chevron = Path::new(|builder| {
                builder.move_to(Point::new(chevron_x - 3.0, center_y - 2.0));
                builder.line_to(Point::new(chevron_x, center_y + 2.0));
                builder.line_to(Point::new(chevron_x + 3.0, center_y - 2.0));
            });
            let lit = hovered == Some(index) || open == Some(index);
            frame.stroke(
                &chevron,
                Stroke::default()
                    .with_color(if lit { TEXT_COLOR } else { DIM_TEXT_COLOR })
                    .with_width(1.5)
                    .with_line_cap(LineCap::Round),
            );
        }
    }
}

/// Lights row `row` of a list that starts at `list`, the way a hovered row of a dropdown is lit.
fn light_row(frame: &mut Frame, list: Rectangle, row: usize) {
    let lit = Rectangle {
        x: list.x + 4.0,
        y: list.y + row as f32 * MENU_ITEM_HEIGHT,
        width: list.width - 8.0,
        height: MENU_ITEM_HEIGHT,
    };
    frame.fill(
        &rounded_rect(lit, DROPDOWN_CORNER_RADIUS),
        LIST_ROW_HOVER_COLOR,
    );
}

/// The text of one row of a list that starts at `list`.
fn list_row_text(frame: &mut Frame, list: Rectangle, row: usize, content: String, color: Color) {
    frame.fill_text(Text {
        content,
        position: Point::new(
            list.x + 12.0,
            list.y + row as f32 * MENU_ITEM_HEIGHT + MENU_ITEM_HEIGHT / 2.0,
        ),
        color,
        size: 13.0,
        vertical_alignment: alignment::Vertical::Center,
        ..Text::default()
    });
}

/// A small arrow pointing right, at the right end of a row that opens a submenu.
fn draw_submenu_arrow(frame: &mut Frame, list: Rectangle, row: usize) {
    let x = list.x + list.width - 16.0;
    let y = list.y + (row as f32 + 0.5) * MENU_ITEM_HEIGHT;
    let arrow = Path::new(|builder| {
        builder.move_to(Point::new(x - 2.0, y - 4.0));
        builder.line_to(Point::new(x + 2.0, y));
        builder.line_to(Point::new(x - 2.0, y + 4.0));
    });
    frame.stroke(
        &arrow,
        Stroke::default()
            .with_color(TEXT_COLOR)
            .with_width(1.5)
            .with_line_cap(LineCap::Round),
    );
}

/// The list under the Presets button: "Save as...", the categories, then the presets with none.
/// A category with its submenu open is highlighted, and the submenu is drawn beside it. The
/// current preset is marked. An empty library says so.
pub(super) fn draw_preset_list(frame: &mut Frame, bounds: Rectangle, graph: &Graph) {
    let Some(menu) = &graph.preset_list else {
        return;
    };
    let library = &menu.library;
    let list = preset_list_bounds(bounds, library);
    draw_shadow(frame, list, MENU_CORNER_RADIUS, FLOAT_SHADOW);
    frame.fill(&rounded_rect(list, MENU_CORNER_RADIUS), MENU_COLOR);
    let (categories, loose) = preset_rows_shown(bounds, library);

    // The row under the cursor lights. A row of the open submenu is lit by the submenu's own draw.
    if graph.cursor_in {
        let row = preset_row_at(bounds, menu, graph.last_screen)
            .and_then(|row| row.list_line(categories));
        if let Some(row) = row {
            light_row(frame, list, row);
        }
    }

    list_row_text(frame, list, 0, "Save as...".to_string(), TEXT_COLOR);
    list_row_text(frame, list, 1, "Init".to_string(), TEXT_COLOR);
    if library.is_empty() {
        list_row_text(
            frame,
            list,
            PRESET_ACTION_ROWS,
            "No presets yet".to_string(),
            DIM_TEXT_COLOR,
        );
    }
    for (index, category) in library.categories.iter().take(categories).enumerate() {
        let row_number = PRESET_ACTION_ROWS + index;
        if menu.open == Some(index) {
            let row = Rectangle {
                x: list.x + 4.0,
                y: list.y + row_number as f32 * MENU_ITEM_HEIGHT,
                width: list.width - 8.0,
                height: MENU_ITEM_HEIGHT,
            };
            frame.fill(
                &rounded_rect(row, DROPDOWN_CORNER_RADIUS),
                LIST_ROW_HOVER_COLOR,
            );
        }
        // A category holding the current preset says so, so it is easy to find again.
        let holds_current = graph.preset_category == category.name;
        let content = if holds_current && graph.preset_name.is_some() {
            format!("{}  *", category.name)
        } else {
            category.name.clone()
        };
        list_row_text(frame, list, row_number, content, TEXT_COLOR);
        draw_submenu_arrow(frame, list, row_number);
    }
    for (index, name) in library.loose.iter().take(loose).enumerate() {
        let current =
            graph.preset_name.as_deref() == Some(name.as_str()) && graph.preset_category.is_empty();
        let content = if current {
            format!("{name}  (current)")
        } else {
            name.clone()
        };
        let row_number = PRESET_ACTION_ROWS + categories + index;
        list_row_text(frame, list, row_number, content, TEXT_COLOR);
    }
    let hidden = (library.categories.len() - categories) + (library.loose.len() - loose);
    if hidden > 0 {
        // Only the first rows fit above the keys.
        frame.fill_text(Text {
            content: format!("{hidden} more not shown"),
            position: Point::new(list.x + 12.0, list.y + list.height + 10.0),
            color: DIM_TEXT_COLOR,
            size: 11.0,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
    }

    if let Some(open) = menu.open {
        draw_submenu(frame, bounds, graph, open);
    }
}

/// The submenu of the category at position `open`: its presets, with the current one marked.
fn draw_submenu(frame: &mut Frame, bounds: Rectangle, graph: &Graph, open: usize) {
    let Some(menu) = &graph.preset_list else {
        return;
    };
    let Some(category) = menu.library.categories.get(open) else {
        return;
    };
    let sub = submenu_bounds(bounds, &menu.library, open);
    draw_shadow(frame, sub, MENU_CORNER_RADIUS, FLOAT_SHADOW);
    frame.fill(&rounded_rect(sub, MENU_CORNER_RADIUS), MENU_COLOR);
    if graph.cursor_in {
        if let Some(PresetRow::Sub(_, row)) = preset_row_at(bounds, menu, graph.last_screen) {
            light_row(frame, sub, row);
        }
    }
    let shown = submenu_rows_shown(bounds, &menu.library, open);
    for (index, name) in category.presets.iter().take(shown).enumerate() {
        let current = graph.preset_name.as_deref() == Some(name.as_str())
            && graph.preset_category == category.name;
        let content = if current {
            format!("{name}  (current)")
        } else {
            name.clone()
        };
        list_row_text(frame, sub, index, content, TEXT_COLOR);
    }
    if category.presets.len() > shown {
        let more = category.presets.len() - shown;
        frame.fill_text(Text {
            content: format!("{more} more not shown"),
            position: Point::new(sub.x + 12.0, sub.y + sub.height + 10.0),
            color: DIM_TEXT_COLOR,
            size: 11.0,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
    }
}

/// The line under the pill that says what a save or load did, for a few seconds.
pub(super) fn draw_status(frame: &mut Frame, bounds: Rectangle, graph: &Graph) {
    let Some((text, since)) = &graph.status else {
        return;
    };
    if since.elapsed() > STATUS_SECONDS {
        return;
    }
    let pill = pill_bounds(bounds);
    frame.fill_text(Text {
        content: text.clone(),
        position: Point::new(pill.x + pill.width / 2.0, pill.y + pill.height + 16.0),
        color: DIM_TEXT_COLOR,
        size: 12.0,
        horizontal_alignment: alignment::Horizontal::Center,
        vertical_alignment: alignment::Vertical::Center,
        ..Text::default()
    });
}

/// The list under an open category. Its rows are the category's node kinds, or a note when the
/// category has none yet.
pub(super) fn draw_category_list(
    frame: &mut Frame,
    bounds: Rectangle,
    category: Category,
    graph: &Graph,
) {
    let list = category_list_bounds(bounds, category);
    draw_shadow(frame, list, MENU_CORNER_RADIUS, FLOAT_SHADOW);
    frame.fill(&rounded_rect(list, MENU_CORNER_RADIUS), MENU_COLOR);
    // A row lights under the cursor, unless it is a kind the graph cannot take another of.
    if graph.cursor_in {
        let hovered = category_row_at(bounds, category, graph.last_screen)
            .filter(|&kind| graph.can_add(kind))
            .and_then(|kind| category.kinds().iter().position(|&k| k == kind));
        if let Some(row) = hovered {
            light_row(frame, list, row);
        }
    }

    let labels: Vec<&str> = if category.kinds().is_empty() {
        vec!["Empty for now"]
    } else {
        category.kinds().iter().map(|kind| kind.title()).collect()
    };
    for (row, label) in labels.iter().enumerate() {
        // A kind that is one per graph is dimmed while the graph has one.
        let available = category
            .kinds()
            .get(row)
            .is_none_or(|&kind| graph.can_add(kind));
        frame.fill_text(Text {
            content: label.to_string(),
            position: Point::new(
                list.x + 12.0,
                list.y + row as f32 * MENU_ITEM_HEIGHT + MENU_ITEM_HEIGHT / 2.0,
            ),
            color: if category.kinds().is_empty() || !available {
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
pub(super) fn draw_menu(
    frame: &mut Frame,
    menu: ContextMenu,
    node: &FlodeNode,
    hovered_row: Option<usize>,
) {
    let bounds = menu_bounds(menu);
    draw_shadow(frame, bounds, MENU_CORNER_RADIUS, FLOAT_SHADOW);
    frame.fill(&rounded_rect(bounds, MENU_CORNER_RADIUS), MENU_COLOR);
    if let Some(row) = hovered_row {
        light_row(frame, bounds, row);
    }
    for (row, item) in menu.items.iter().enumerate() {
        frame.fill_text(Text {
            content: item.label(node).to_string(),
            position: Point::new(
                bounds.x + 12.0,
                bounds.y + row as f32 * MENU_ITEM_HEIGHT + MENU_ITEM_HEIGHT / 2.0,
            ),
            color: TEXT_COLOR,
            size: 14.0,
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

/// The quick add box: a search field with the typed text and a caret, and under it a row for each
/// matching node. The lit row is the one Enter adds. With no match, one dim row says so.
pub(super) fn draw_quick_add(frame: &mut Frame, bounds: Rectangle, graph: &Graph) {
    let Some(open) = &graph.quick_add else {
        return;
    };
    let results = graph.quick_add_results();
    let area = quick_add_bounds(bounds, open.screen, results.len());
    draw_shadow(frame, area, MENU_CORNER_RADIUS, FLOAT_SHADOW);
    frame.fill(&rounded_rect(area, MENU_CORNER_RADIUS), MENU_COLOR);

    let field = Rectangle {
        x: area.x + 6.0,
        y: area.y + 6.0,
        width: area.width - 12.0,
        height: QUICK_ADD_FIELD_HEIGHT - 12.0,
    };
    frame.fill(&rounded_rect(field, DROPDOWN_CORNER_RADIUS), DROPDOWN_COLOR);
    let (shown, shade) = if open.query.is_empty() {
        ("Search nodes|".to_string(), DIM_TEXT_COLOR)
    } else {
        (format!("{}|", open.query), TEXT_COLOR)
    };
    let text = |frame: &mut Frame, content: String, y: f32, color: Color| {
        frame.fill_text(Text {
            content,
            position: Point::new(area.x + 14.0, y),
            color,
            size: 13.0,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
    };
    text(frame, shown, field.y + field.height / 2.0, shade);

    let list = Rectangle {
        y: area.y + QUICK_ADD_FIELD_HEIGHT,
        height: area.height - QUICK_ADD_FIELD_HEIGHT,
        ..area
    };
    if results.is_empty() {
        text(
            frame,
            "No match".to_string(),
            list.y + MENU_ITEM_HEIGHT / 2.0,
            DIM_TEXT_COLOR,
        );
        return;
    }
    light_row(frame, list, open.selected.min(results.len() - 1));
    for (row, kind) in results.iter().enumerate() {
        text(
            frame,
            kind.title().to_string(),
            list.y + row as f32 * MENU_ITEM_HEIGHT + MENU_ITEM_HEIGHT / 2.0,
            TEXT_COLOR,
        );
    }
}
