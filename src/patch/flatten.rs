//! Flattening: a graph with groups becomes a plain graph the compiler already understands. Each
//! group node is replaced, in place, by the nodes of the graph it holds. The group edge nodes
//! vanish, and a wire that crossed a group pin runs straight from the node that made the signal
//! to the node that takes it. The audio engine never sees a group.

use crate::nodes::connection::Connection;
use crate::nodes::{FlodeNode, NodeKind, Port};

/// The most pin hops a signal may take on its way to a real node. A loop of bare pin wires
/// exceeds it and makes no signal.
const MAX_HOPS: usize = 64;

/// A graph with no groups in it.
pub struct Flat {
    /// Copies of the nodes that are not group parts, in graph order. A group's nodes come where
    /// the group node was.
    pub nodes: Vec<FlodeNode>,
    pub links: Vec<Connection>,
    /// Where each flat node came from. The path is the index of every group node on the way down,
    /// then the node's index in the graph that holds it.
    pub paths: Vec<Vec<usize>>,
}

/// One graph in the tree of groups: the root, or the graph a group node holds.
struct Level<'a> {
    nodes: &'a [FlodeNode],
    links: &'a [Connection],
    /// The level that holds this one's group node, and that node's index there.
    parent: Option<(usize, usize)>,
    /// The flat index of each node that is not a group part.
    flat: Vec<Option<usize>>,
    /// The level of each group node's graph.
    kids: Vec<Option<usize>>,
}

/// Flattens the graph made of `nodes` and `links`. A graph with no group nodes comes back as
/// copies of the same nodes and wires.
pub fn flatten(nodes: &[FlodeNode], links: &[Connection]) -> Flat {
    let mut flat = Flat {
        nodes: Vec::new(),
        links: Vec::new(),
        paths: Vec::new(),
    };
    let mut levels = Vec::new();
    place(nodes, links, None, &mut Vec::new(), &mut levels, &mut flat);
    for (level, graph) in levels.iter().enumerate() {
        for link in graph.links {
            // A wire into a group pin or an edge node is read from the other end, by whatever
            // takes the signal on the far side.
            let Some(&Some(to)) = graph.flat.get(link.to) else {
                continue;
            };
            let Some((from, out)) = source(&levels, level, link.from, link.out, 0) else {
                continue;
            };
            flat.links.push(Connection {
                from,
                out,
                to,
                input: link.input,
            });
        }
    }
    flat
}

/// Adds a graph to `levels`, and every node of it that is not a group part to `flat`. A group node
/// adds its own graph right there, so its nodes keep the group's place in the order.
fn place<'a>(
    nodes: &'a [FlodeNode],
    links: &'a [Connection],
    parent: Option<(usize, usize)>,
    path: &mut Vec<usize>,
    levels: &mut Vec<Level<'a>>,
    flat: &mut Flat,
) -> usize {
    let id = levels.len();
    levels.push(Level {
        nodes,
        links,
        parent,
        flat: vec![None; nodes.len()],
        kids: vec![None; nodes.len()],
    });
    for (index, node) in nodes.iter().enumerate() {
        match node.kind {
            NodeKind::Group => {
                if let Some(group) = &node.group {
                    path.push(index);
                    let kid = place(
                        &group.nodes,
                        &group.links,
                        Some((id, index)),
                        path,
                        levels,
                        flat,
                    );
                    path.pop();
                    levels[id].kids[index] = Some(kid);
                }
            }
            NodeKind::GroupInput | NodeKind::GroupOutput => {}
            _ => {
                levels[id].flat[index] = Some(flat.nodes.len());
                flat.nodes.push(node.duplicate(node.position));
                let mut at = path.clone();
                at.push(index);
                flat.paths.push(at);
            }
        }
    }
    id
}

/// Finds the link that ends on pin `pin` of node `to`.
fn into_pin(links: &[Connection], to: usize, pin: usize) -> Option<&Connection> {
    links
        .iter()
        .find(|c| c.to == to && matches!(c.input, Port::Pin(_, n) if n == pin))
}

/// The node and output port that make the signal on output `port` of node `node` in `level`. A
/// real node makes its own. A group node's output pin is whatever feeds the same pin on its
/// output edge, and an input edge node's pin is whatever feeds the same pin on the group node
/// outside. None means nothing feeds the pin.
fn source(
    levels: &[Level],
    level: usize,
    node: usize,
    port: Port,
    hops: usize,
) -> Option<(usize, Port)> {
    if hops > MAX_HOPS {
        return None;
    }
    let graph = &levels[level];
    if let Some(&Some(found)) = graph.flat.get(node) {
        return Some((found, port));
    }
    let Port::Pin(_, pin) = port else {
        return None;
    };
    match graph.nodes.get(node)?.kind {
        NodeKind::Group => {
            let inner = (*graph.kids.get(node)?)?;
            let edge = levels[inner]
                .nodes
                .iter()
                .position(|n| n.kind == NodeKind::GroupOutput)?;
            let link = into_pin(levels[inner].links, edge, pin)?;
            source(levels, inner, link.from, link.out, hops + 1)
        }
        NodeKind::GroupInput => {
            let (outer, group) = graph.parent?;
            let link = into_pin(levels[outer].links, group, pin)?;
            source(levels, outer, link.from, link.out, hops + 1)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::group::GroupPin;
    use crate::nodes::Signal;
    use nih_plug_iced::Point;

    fn node(kind: NodeKind) -> FlodeNode {
        FlodeNode::new(kind, Point::new(0.0, 0.0))
    }

    fn audio(from: usize, to: usize) -> Connection {
        Connection {
            from,
            out: Port::Audio,
            to,
            input: Port::Audio,
        }
    }

    fn pin(n: usize) -> Port {
        Port::Pin(Signal::Audio, n)
    }

    /// A group with one audio pin each way and a filter between them.
    fn filter_group() -> FlodeNode {
        let mut group = node(NodeKind::Group);
        group.inputs = vec![GroupPin::new("Audio", Signal::Audio)];
        group.outputs = vec![GroupPin::new("Audio", Signal::Audio)];
        let data = group.group.as_mut().unwrap();
        data.nodes[0].outputs = group.inputs.clone();
        data.nodes[1].inputs = group.outputs.clone();
        data.nodes.push(node(NodeKind::Filter));
        data.links = vec![
            Connection {
                from: 0,
                out: pin(0),
                to: 2,
                input: Port::Audio,
            },
            Connection {
                from: 2,
                out: Port::Audio,
                to: 1,
                input: pin(0),
            },
        ];
        group
    }

    #[test]
    fn a_graph_without_groups_comes_back_unchanged() {
        let nodes = [node(NodeKind::Oscillator), node(NodeKind::Reverb)];
        let flat = flatten(&nodes, &[audio(0, 1)]);
        assert_eq!(flat.nodes.len(), 2);
        assert_eq!(flat.links, vec![audio(0, 1)]);
        assert_eq!(flat.paths, vec![vec![0], vec![1]]);
    }

    #[test]
    fn a_group_expands_in_place_and_its_wires_run_straight_through() {
        let nodes = [
            node(NodeKind::Oscillator),
            filter_group(),
            node(NodeKind::Reverb),
        ];
        let links = [
            Connection {
                from: 0,
                out: Port::Audio,
                to: 1,
                input: pin(0),
            },
            Connection {
                from: 1,
                out: pin(0),
                to: 2,
                input: Port::Audio,
            },
        ];
        let flat = flatten(&nodes, &links);
        let kinds: Vec<NodeKind> = flat.nodes.iter().map(|n| n.kind).collect();
        assert_eq!(
            kinds,
            vec![NodeKind::Oscillator, NodeKind::Filter, NodeKind::Reverb]
        );
        assert_eq!(flat.paths[1], vec![1, 2]);
        assert!(flat.links.contains(&audio(0, 1)));
        assert!(flat.links.contains(&audio(1, 2)));
        assert_eq!(flat.links.len(), 2);
    }

    #[test]
    fn a_pin_wired_straight_across_connects_the_outside() {
        let mut group = node(NodeKind::Group);
        group.inputs = vec![GroupPin::new("Audio", Signal::Audio)];
        group.outputs = vec![GroupPin::new("Audio", Signal::Audio)];
        let data = group.group.as_mut().unwrap();
        data.nodes[0].outputs = group.inputs.clone();
        data.nodes[1].inputs = group.outputs.clone();
        data.links = vec![Connection {
            from: 0,
            out: pin(0),
            to: 1,
            input: pin(0),
        }];
        let nodes = [node(NodeKind::Oscillator), group, node(NodeKind::Reverb)];
        let links = [
            Connection {
                from: 0,
                out: Port::Audio,
                to: 1,
                input: pin(0),
            },
            Connection {
                from: 1,
                out: pin(0),
                to: 2,
                input: Port::Audio,
            },
        ];
        let flat = flatten(&nodes, &links);
        assert_eq!(flat.nodes.len(), 2);
        assert_eq!(flat.links, vec![audio(0, 1)]);
    }

    #[test]
    fn a_loop_of_bare_pin_wires_makes_no_signal() {
        let mut group = node(NodeKind::Group);
        group.inputs = vec![GroupPin::new("Audio", Signal::Audio)];
        group.outputs = vec![GroupPin::new("Audio", Signal::Audio)];
        let data = group.group.as_mut().unwrap();
        data.nodes[0].outputs = group.inputs.clone();
        data.nodes[1].inputs = group.outputs.clone();
        data.links = vec![Connection {
            from: 0,
            out: pin(0),
            to: 1,
            input: pin(0),
        }];
        let nodes = [group, node(NodeKind::Reverb)];
        let links = [
            Connection {
                from: 0,
                out: pin(0),
                to: 0,
                input: pin(0),
            },
            Connection {
                from: 0,
                out: pin(0),
                to: 1,
                input: Port::Audio,
            },
        ];
        let flat = flatten(&nodes, &links);
        assert_eq!(flat.nodes.len(), 1);
        assert!(flat.links.is_empty());
    }
}
