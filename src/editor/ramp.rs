//! The wavetable node's ramp: a bar with a colored region for each connected frame input, a handle
//! between neighbouring regions, and a dot where Position is. This module holds where those sit,
//! which one a press grabs, and how a drag on a handle trades share between two regions.

use crate::dsp::table::{BlendMode, Ramp};
use crate::editor::knob::{self, Press};
use crate::nodes::knobs::KnobId;
use crate::nodes::viewer::Live;
use crate::nodes::wavetable::{blend_index, MAX_FRAMES};
use crate::nodes::{FlodeNode, NodeKind, Port};
use nih_plug_iced::widget::canvas::event;
use nih_plug_iced::{Point, Rectangle};

use super::canvas::{Message, NodeEditor};
use super::curve::HANDLE_HIT_RADIUS;
use super::graph::{Graph, Interaction, Terminal};

/// How far down the ramp's box the bar starts. The position dot sits in the space above it.
pub(super) const BAR_TOP: f32 = 18.0;
/// How tall the bar is.
pub(super) const BAR_HEIGHT: f32 = 24.0;
/// How far a handle reaches above and below the bar.
pub(super) const HANDLE_OVERHANG: f32 = 3.0;
/// How wide a handle is.
pub(super) const HANDLE_WIDTH: f32 = 4.0;

/// Where a ramp's regions sit: the bar, the frame slots that have a region, and the layout of
/// those regions along it.
#[derive(Debug, Clone, Copy)]
pub(super) struct RampLayout {
    /// The bar, in editor coordinates.
    pub(super) bar: Rectangle,
    /// The pin number, from 0, of each region in pin order. The first `count` are used.
    slots: [usize; MAX_FRAMES],
    count: usize,
    /// The regions along the bar. With nothing connected it holds one region that is not drawn.
    pub(super) ramp: Ramp,
}

impl RampLayout {
    /// How many regions the ramp has, which is how many frame inputs are connected.
    pub(super) fn count(&self) -> usize {
        self.count
    }

    /// The frame slots that have a region, in pin order.
    pub(super) fn slots(&self) -> &[usize] {
        &self.slots[..self.count]
    }

    /// Where region `region` sits on the bar.
    pub(super) fn region_rect(&self, region: usize) -> Rectangle {
        let (start, end) = self.ramp.span(region);
        Rectangle {
            x: self.bar.x + start * self.bar.width,
            y: self.bar.y,
            width: (end - start) * self.bar.width,
            height: self.bar.height,
        }
    }

    /// The x of the handle between region `boundary` and the one after it.
    pub(super) fn handle_x(&self, boundary: usize) -> f32 {
        self.bar.x + self.ramp.boundaries()[boundary] * self.bar.width
    }

    /// The box a handle is drawn in, taller than the bar.
    pub(super) fn handle_rect(&self, boundary: usize) -> Rectangle {
        Rectangle {
            x: self.handle_x(boundary) - HANDLE_WIDTH / 2.0,
            y: self.bar.y - HANDLE_OVERHANG,
            width: HANDLE_WIDTH,
            height: self.bar.height + 2.0 * HANDLE_OVERHANG,
        }
    }

    /// The x on the bar where a Position, from 0.0 to 1.0, is.
    pub(super) fn position_x(&self, position: f32) -> f32 {
        self.bar.x + position.clamp(0.0, 1.0) * self.bar.width
    }

    /// The handle under `point`, as the region it follows. Where handles overlap, the nearest
    /// wins, and the later one wins a tie, since it is drawn last.
    pub(super) fn handle_at(&self, point: Point) -> Option<usize> {
        let reach = HANDLE_OVERHANG + HANDLE_HIT_RADIUS / 2.0;
        if point.y < self.bar.y - reach || point.y > self.bar.y + self.bar.height + reach {
            return None;
        }
        (0..self.count.saturating_sub(1))
            .map(|boundary| (boundary, (self.handle_x(boundary) - point.x).abs()))
            .filter(|(_, distance)| *distance <= HANDLE_HIT_RADIUS)
            .min_by(|(a, da), (b, db)| {
                da.partial_cmp(db)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.cmp(a))
            })
            .map(|(boundary, _)| boundary)
    }

    /// The frame slot whose region is under `point`, if any.
    pub(super) fn slot_at(&self, point: Point) -> Option<usize> {
        if self.count == 0 || !self.bar.contains(point) {
            return None;
        }
        (0..self.count)
            .find(|&region| {
                let rect = self.region_rect(region);
                point.x < rect.x + rect.width || region + 1 == self.count
            })
            .map(|region| self.slots[region])
    }
}

/// The frame slots of node `index` that have a wire into them, in pin order.
pub(super) fn connected_slots(graph: &Graph, index: usize) -> Vec<usize> {
    let mut slots: Vec<usize> = graph
        .connections
        .iter()
        .filter(|c| c.to == index)
        .filter_map(|c| match c.input {
            Port::Frame(slot) if slot < MAX_FRAMES => Some(slot),
            _ => None,
        })
        .collect();
    slots.sort_unstable();
    slots.dedup();
    slots
}

/// The relative share each of `slots` has, from the node's share knobs.
fn shares_of(node: &FlodeNode, slots: &[usize]) -> Vec<f32> {
    slots
        .iter()
        .map(|&slot| node.plain(KnobId::share(slot)))
        .collect()
}

/// The ramp of node `index`. None if the node is not a wavetable, or its ramp is folded away.
pub(super) fn ramp_layout(graph: &Graph, index: usize) -> Option<RampLayout> {
    let node = graph.nodes.get(index)?;
    if node.kind != NodeKind::Wavetable {
        return None;
    }
    let bounds = node.ramp_bounds()?;
    let connected = connected_slots(graph, index);
    let mut slots = [0; MAX_FRAMES];
    slots[..connected.len()].copy_from_slice(&connected);
    let ramp = Ramp::new(
        &shares_of(node, &connected),
        BlendMode::from_index(blend_index(node)),
        node.plain(KnobId::WtBlendWidth),
    );
    Some(RampLayout {
        bar: Rectangle {
            x: bounds.x,
            y: bounds.y + BAR_TOP,
            width: bounds.width,
            height: BAR_HEIGHT,
        },
        slots,
        count: connected.len(),
        ramp,
    })
}

/// The Position the ramp shows, with a wire's value if one drives it.
pub(super) fn live_position(live: &Live) -> f32 {
    live.value(KnobId::WtPosition).clamp(0.0, 1.0)
}

/// The smallest and largest a share can be, from its knob's range.
fn share_range() -> (f32, f32) {
    let id = KnobId::share(0);
    (id.to_plain(0.0), id.to_plain(1.0))
}

/// Moves the boundary after region `boundary` to `fraction` of the way along the bar, by trading
/// share between the regions on its two sides. The total stays the same, so no other boundary
/// moves, and every share stays in `min..=max`.
pub(super) fn transfer_share(
    shares: &mut [f32],
    boundary: usize,
    fraction: f32,
    min: f32,
    max: f32,
) {
    if boundary + 1 >= shares.len() {
        return;
    }
    let total: f32 = shares.iter().sum();
    let before: f32 = shares[..boundary].iter().sum();
    let pair = shares[boundary] + shares[boundary + 1];
    let lowest = min.max(pair - max);
    let highest = max.min(pair - min);
    if lowest > highest {
        return;
    }
    let left = (fraction.clamp(0.0, 1.0) * total - before).clamp(lowest, highest);
    shares[boundary] = left;
    shares[boundary + 1] = pair - left;
}

impl Graph {
    /// Drags the handle after region `boundary` of wavetable node `index` to `point`. Only the
    /// two share knobs beside the handle change.
    pub(super) fn drag_ramp_handle(&mut self, index: usize, boundary: usize, point: Point) {
        let Some(layout) = ramp_layout(self, index) else {
            return;
        };
        let slots = layout.slots();
        if boundary + 1 >= slots.len() {
            return;
        }
        let node = &mut self.nodes[index];
        let mut shares = shares_of(node, slots);
        let fraction = (point.x - layout.bar.x) / layout.bar.width;
        let (min, max) = share_range();
        transfer_share(&mut shares, boundary, fraction, min, max);
        for region in [boundary, boundary + 1] {
            let id = KnobId::share(slots[region]);
            node.set_value(id, id.to_normalized(shares[region]));
        }
    }

    /// The frame input the cursor links to at `point`: the slot of the pin under it, or of the
    /// region under it, when that slot has a region on a wavetable's ramp. The region and the pin
    /// of a slot light together.
    pub(super) fn ramp_hover_at(&self, point: Point) -> Option<(usize, usize)> {
        if let Some(Terminal::In(index, Port::Frame(slot))) = self.terminal_at(point) {
            let layout = ramp_layout(self, index)?;
            return layout.slots().contains(&slot).then_some((index, slot));
        }
        let index = self.node_at(point)?;
        let slot = ramp_layout(self, index)?.slot_at(point)?;
        Some((index, slot))
    }
}

impl NodeEditor<'_> {
    /// A left press on node `index`'s ramp. A press on a handle starts dragging it, and a double
    /// click on the bar or a handle puts every share back to equal. A press elsewhere on the bar
    /// does nothing else. None if the press is not on the ramp.
    pub(super) fn press_ramp(
        &mut self,
        index: usize,
        point: Point,
        screen: Point,
    ) -> Option<(event::Status, Option<Message>)> {
        let layout = ramp_layout(self.graph, index)?;
        let handle = layout.handle_at(point);
        if handle.is_none() && !layout.bar.contains(point) {
            return None;
        }
        self.graph.select(index);
        let node = &mut self.graph.nodes[index];
        let first = KnobId::share(0);
        let current = node.value(first);
        let state = &mut node.knob_states[first as usize];
        match knob::press(state, current, screen) {
            Press::Reset => {
                for slot in 0..MAX_FRAMES {
                    node.reset_value(KnobId::share(slot));
                }
                self.graph.interaction = Interaction::Idle;
            }
            Press::Drag => {
                self.graph.interaction = match handle {
                    Some(boundary) => Interaction::RampHandle {
                        node: index,
                        boundary,
                    },
                    None => Interaction::Idle,
                };
            }
        }
        Some((event::Status::Captured, Some(Message::GraphChanged)))
    }
}
