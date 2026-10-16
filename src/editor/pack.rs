//! Auto pack: lays a group of nodes out left to right by how they are wired, so signal flows
//! the way the graph is read. It looks only at the wires between the nodes it is given.

use nih_plug_iced::{Point, Size};

/// The space between one column of nodes and the next, wide enough to see the wires.
const COLUMN_GAP: f32 = 60.0;
/// The space between nodes in a column.
const ROW_GAP: f32 = 28.0;

/// New top-left positions for nodes with the given `sizes`, laid out in columns.
///
/// `at` holds the nodes' current top-left corners, and `wires` holds `(from, to)` pairs of indices
/// into both. A node sits one column right of the furthest-left node that feeds it, so a source is
/// in the first column and the end of a chain in the last. A wire loop is cut where it closes, and
/// a node with no wires to the group goes in the first column. Each column is centered on the
/// middle of the group's old height. Within a column, nodes follow the height of the nodes that feed
/// them, so wires cross less. The group's left edge stays where it was.
pub(super) fn pack(at: &[Point], sizes: &[Size], wires: &[(usize, usize)]) -> Vec<Point> {
    let count = at.len();
    if count == 0 {
        return Vec::new();
    }

    // A node's column is the longest chain of wires leading to it. Repeating once per node settles
    // every chain, and a loop stops growing at the cap.
    let mut column = vec![0usize; count];
    for _ in 0..count {
        for &(from, to) in wires {
            if from != to {
                column[to] = column[to].max(column[from] + 1).min(count - 1);
            }
        }
    }
    // Empty columns close up.
    let mut used: Vec<usize> = column.clone();
    used.sort_unstable();
    used.dedup();
    for value in &mut column {
        *value = used.iter().position(|u| u == value).unwrap_or(0);
    }

    let left = at.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let (top, bottom) = at.iter().zip(sizes).fold(
        (f32::INFINITY, f32::NEG_INFINITY),
        |(top, bottom), (p, s)| (top.min(p.y), bottom.max(p.y + s.height)),
    );
    let middle = (top + bottom) / 2.0;

    let mut placed = vec![Point::new(0.0, 0.0); count];
    let mut x = left;
    for current in 0..used.len() {
        let mut members: Vec<usize> = (0..count).filter(|&n| column[n] == current).collect();
        // Sort by the middle of the nodes that feed each one, or by its old height if none does.
        let key = |n: usize| {
            let feeders: Vec<f32> = wires
                .iter()
                .filter(|&&(from, to)| to == n && column[from] < current)
                .map(|&(from, _)| placed[from].y + sizes[from].height / 2.0)
                .collect();
            if feeders.is_empty() {
                at[n].y + sizes[n].height / 2.0
            } else {
                feeders.iter().sum::<f32>() / feeders.len() as f32
            }
        };
        members.sort_by(|&a, &b| key(a).total_cmp(&key(b)));

        let height: f32 = members.iter().map(|&n| sizes[n].height).sum::<f32>()
            + ROW_GAP * (members.len() - 1) as f32;
        let mut y = middle - height / 2.0;
        for &n in &members {
            placed[n] = Point::new(x, y);
            y += sizes[n].height + ROW_GAP;
        }
        let width = members.iter().map(|&n| sizes[n].width).fold(0.0, f32::max);
        x += width + COLUMN_GAP;
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size() -> Size {
        Size::new(200.0, 100.0)
    }

    #[test]
    fn a_chain_runs_left_to_right() {
        let at = [
            Point::new(500.0, 0.0),
            Point::new(0.0, 300.0),
            Point::new(250.0, 100.0),
        ];
        // 0 feeds 1, and 1 feeds 2.
        let placed = pack(&at, &[size(); 3], &[(0, 1), (1, 2)]);
        assert!(placed[0].x < placed[1].x && placed[1].x < placed[2].x);
        // The group keeps its left edge.
        assert_eq!(placed[0].x, 0.0);
    }

    #[test]
    fn a_merge_goes_after_both_of_its_inputs() {
        let at = [
            Point::new(0.0, 0.0),
            Point::new(0.0, 200.0),
            Point::new(0.0, 100.0),
        ];
        let placed = pack(&at, &[size(); 3], &[(0, 2), (1, 2)]);
        assert_eq!(placed[0].x, placed[1].x);
        assert!(placed[2].x > placed[0].x);
        // The two inputs do not overlap.
        assert!((placed[0].y - placed[1].y).abs() >= 100.0 + ROW_GAP);
    }

    #[test]
    fn a_loop_still_finishes() {
        let at = [Point::new(0.0, 0.0), Point::new(300.0, 0.0)];
        let placed = pack(&at, &[size(); 2], &[(0, 1), (1, 0)]);
        assert_eq!(placed.len(), 2);
        assert!(placed.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
    }

    #[test]
    fn no_two_nodes_overlap() {
        let at = [Point::new(0.0, 0.0); 5];
        let placed = pack(&at, &[size(); 5], &[(0, 1), (0, 2), (1, 3), (2, 3), (3, 4)]);
        for a in 0..5 {
            for b in a + 1..5 {
                let apart_x = (placed[a].x - placed[b].x).abs() >= 200.0;
                let apart_y = (placed[a].y - placed[b].y).abs() >= 100.0;
                assert!(apart_x || apart_y, "{a} and {b}");
            }
        }
    }

    #[test]
    fn unwired_nodes_go_in_the_first_column() {
        let at = [Point::new(10.0, 0.0), Point::new(400.0, 0.0)];
        let placed = pack(&at, &[size(); 2], &[]);
        assert_eq!(placed[0].x, 10.0);
        assert_eq!(placed[1].x, 10.0);
    }
}
