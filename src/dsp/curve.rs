//! A drawn curve: a short list of points joined by straight lines, for a wave the user draws. It is
//! plain data with no heap, so a patch can hold one and the audio thread can copy it.

/// The most points a curve holds.
pub const MAX_CURVE_POINTS: usize = 16;

/// The least distance in x between two neighbouring points.
pub const MIN_GAP: f32 = 0.01;

/// Points `[x, y]`, both from 0.0 to 1.0. The x values increase. The first point is at x 0.0 and
/// the last at x 1.0, and there are at least two points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Curve {
    points: [[f32; 2]; MAX_CURVE_POINTS],
    len: u8,
}

impl Curve {
    /// A triangle that starts at the bottom, peaks in the middle, and returns to the bottom.
    pub const fn triangle() -> Self {
        let mut points = [[0.0; 2]; MAX_CURVE_POINTS];
        points[1] = [0.5, 1.0];
        points[2] = [1.0, 0.0];
        Self { points, len: 3 }
    }

    /// The curve through `points`, or None if they do not make a valid curve. That means fewer
    /// than 2 or more than `MAX_CURVE_POINTS` of them, a value outside 0.0 to 1.0, a first x that
    /// is not 0.0 or a last x that is not 1.0, or x values that do not increase.
    pub fn from_points(points: &[[f32; 2]]) -> Option<Self> {
        if points.len() < 2 || points.len() > MAX_CURVE_POINTS {
            return None;
        }
        let in_range = |v: f32| (0.0..=1.0).contains(&v);
        if !points.iter().all(|p| in_range(p[0]) && in_range(p[1])) {
            return None;
        }
        if points[0][0] != 0.0 || points[points.len() - 1][0] != 1.0 {
            return None;
        }
        if points.windows(2).any(|pair| pair[1][0] <= pair[0][0]) {
            return None;
        }
        let mut stored = [[0.0; 2]; MAX_CURVE_POINTS];
        stored[..points.len()].copy_from_slice(points);
        Some(Self {
            points: stored,
            len: points.len() as u8,
        })
    }

    /// The points, left to right.
    pub fn points(&self) -> &[[f32; 2]] {
        &self.points[..self.len as usize]
    }

    /// How many points there are.
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// True while no more points fit.
    pub fn is_full(&self) -> bool {
        self.len() >= MAX_CURVE_POINTS
    }

    /// The height at `x`, from 0.0 to 1.0, straight between the two points around it. An `x`
    /// outside 0.0 to 1.0 reads the nearest end.
    pub fn eval(&self, x: f32) -> f32 {
        let points = self.points();
        let x = x.clamp(0.0, 1.0);
        let next = points
            .iter()
            .position(|p| p[0] >= x)
            .unwrap_or(points.len() - 1)
            .max(1);
        let (a, b) = (points[next - 1], points[next]);
        let span = b[0] - a[0];
        if span <= 0.0 {
            return b[1];
        }
        a[1] + (b[1] - a[1]) * ((x - a[0]) / span).clamp(0.0, 1.0)
    }

    /// Adds a point at `(x, y)`, with both clamped to 0.0 to 1.0, and returns its index. Nothing
    /// is added, and the result is None, when the curve is full or `x` is closer than `MIN_GAP`
    /// to a neighbour.
    pub fn insert(&mut self, x: f32, y: f32) -> Option<usize> {
        if self.is_full() {
            return None;
        }
        let (x, y) = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
        let at = self.points().iter().position(|p| p[0] > x)?;
        if x - self.points[at - 1][0] < MIN_GAP || self.points[at][0] - x < MIN_GAP {
            return None;
        }
        let len = self.len();
        self.points.copy_within(at..len, at + 1);
        self.points[at] = [x, y];
        self.len += 1;
        Some(at)
    }

    /// Removes point `index`. Returns false, and changes nothing, for an end point or when only
    /// two points are left.
    pub fn remove(&mut self, index: usize) -> bool {
        let len = self.len();
        if len <= 2 || index == 0 || index >= len - 1 {
            return false;
        }
        self.points.copy_within(index + 1..len, index);
        self.len -= 1;
        true
    }

    /// Moves point `index` to `(x, y)`. The y clamps to 0.0 to 1.0. An end point keeps its x. An
    /// inner point stays at least `MIN_GAP` from each neighbour.
    pub fn move_point(&mut self, index: usize, x: f32, y: f32) {
        let len = self.len();
        if index >= len {
            return;
        }
        let y = y.clamp(0.0, 1.0);
        let x = if index == 0 || index == len - 1 {
            self.points[index][0]
        } else {
            let low = self.points[index - 1][0] + MIN_GAP;
            let high = self.points[index + 1][0] - MIN_GAP;
            if low > high {
                self.points[index][0]
            } else {
                x.clamp(low, high)
            }
        };
        self.points[index] = [x, y];
    }
}

impl Default for Curve {
    fn default() -> Self {
        Self::triangle()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_a_triangle() {
        let curve = Curve::default();
        assert_eq!(curve.points(), &[[0.0, 0.0], [0.5, 1.0], [1.0, 0.0]]);
    }

    #[test]
    fn eval_is_exact_at_the_points() {
        let curve = Curve::default();
        assert_eq!(curve.eval(0.0), 0.0);
        assert_eq!(curve.eval(0.5), 1.0);
        assert_eq!(curve.eval(1.0), 0.0);
    }

    #[test]
    fn eval_is_linear_between_points() {
        let curve = Curve::default();
        assert!((curve.eval(0.25) - 0.5).abs() < 1e-6);
        assert!((curve.eval(0.75) - 0.5).abs() < 1e-6);
        assert!((curve.eval(0.1) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn eval_clamps_x() {
        let curve = Curve::from_points(&[[0.0, 0.3], [1.0, 0.8]]).unwrap();
        assert_eq!(curve.eval(-2.0), 0.3);
        assert_eq!(curve.eval(5.0), 0.8);
    }

    #[test]
    fn insert_keeps_the_points_sorted() {
        let mut curve = Curve::default();
        assert_eq!(curve.insert(0.75, 0.9), Some(2));
        assert_eq!(curve.insert(0.25, 0.1), Some(1));
        let xs: Vec<f32> = curve.points().iter().map(|p| p[0]).collect();
        assert_eq!(xs, vec![0.0, 0.25, 0.5, 0.75, 1.0]);
        assert_eq!(curve.points()[1], [0.25, 0.1]);
        assert_eq!(curve.points()[3], [0.75, 0.9]);
    }

    #[test]
    fn insert_clamps_y() {
        let mut curve = Curve::default();
        curve.insert(0.25, 4.0);
        assert_eq!(curve.points()[1], [0.25, 1.0]);
    }

    #[test]
    fn insert_refuses_a_point_too_close_to_a_neighbour() {
        let mut curve = Curve::default();
        assert_eq!(curve.insert(0.505, 0.5), None);
        assert_eq!(curve.insert(0.995, 0.5), None);
        assert_eq!(curve.insert(0.005, 0.5), None);
        assert_eq!(curve.len(), 3);
        assert!(curve.insert(0.52, 0.5).is_some());
    }

    #[test]
    fn insert_refuses_past_sixteen_points() {
        let mut curve = Curve::default();
        for n in 1..=13 {
            assert!(curve.insert(n as f32 * 0.06, 0.5).is_some(), "{n}");
        }
        assert_eq!(curve.len(), MAX_CURVE_POINTS);
        assert_eq!(curve.insert(0.9, 0.5), None);
        assert_eq!(curve.len(), MAX_CURVE_POINTS);
    }

    #[test]
    fn remove_takes_out_an_inner_point() {
        let mut curve = Curve::default();
        curve.insert(0.25, 0.5);
        assert!(curve.remove(1));
        assert_eq!(curve.points(), &[[0.0, 0.0], [0.5, 1.0], [1.0, 0.0]]);
    }

    #[test]
    fn remove_refuses_the_end_points() {
        let mut curve = Curve::default();
        curve.insert(0.25, 0.5);
        assert!(!curve.remove(0));
        assert!(!curve.remove(3));
        assert!(!curve.remove(9));
        assert_eq!(curve.len(), 4);
    }

    #[test]
    fn remove_refuses_below_two_points() {
        let mut curve = Curve::from_points(&[[0.0, 0.0], [1.0, 1.0]]).unwrap();
        assert!(!curve.remove(1));
        assert_eq!(curve.len(), 2);
    }

    #[test]
    fn an_end_point_moves_only_in_y() {
        let mut curve = Curve::default();
        curve.move_point(0, 0.4, 0.7);
        curve.move_point(2, 0.4, 2.0);
        assert_eq!(curve.points()[0], [0.0, 0.7]);
        assert_eq!(curve.points()[2], [1.0, 1.0]);
    }

    #[test]
    fn an_inner_point_stays_clear_of_its_neighbours() {
        let mut curve = Curve::default();
        curve.insert(0.25, 0.5);
        curve.move_point(2, 0.0, 0.5);
        assert!((curve.points()[2][0] - (0.25 + MIN_GAP)).abs() < 1e-6);
        curve.move_point(2, 5.0, 0.5);
        assert!((curve.points()[2][0] - (1.0 - MIN_GAP)).abs() < 1e-6);
    }

    #[test]
    fn an_inner_point_clamps_y() {
        let mut curve = Curve::default();
        curve.move_point(1, 0.5, -3.0);
        assert_eq!(curve.points()[1], [0.5, 0.0]);
    }

    #[test]
    fn from_points_checks_the_points() {
        assert!(Curve::from_points(&[[0.0, 0.0]]).is_none());
        assert!(Curve::from_points(&[[0.1, 0.0], [1.0, 0.0]]).is_none());
        assert!(Curve::from_points(&[[0.0, 0.0], [0.9, 0.0]]).is_none());
        assert!(Curve::from_points(&[[0.0, 0.0], [0.6, 0.0], [0.4, 0.0], [1.0, 0.0]]).is_none());
        assert!(Curve::from_points(&[[0.0, 0.0], [0.5, 1.5], [1.0, 0.0]]).is_none());
        assert!(Curve::from_points(&[[0.0, 0.0], [0.5, f32::NAN], [1.0, 0.0]]).is_none());
        let many: Vec<[f32; 2]> = (0..17).map(|n| [n as f32 / 16.0, 0.5]).collect();
        assert!(Curve::from_points(&many).is_none());
        assert!(Curve::from_points(&many[..16]).is_none());
        let sixteen: Vec<[f32; 2]> = (0..16).map(|n| [n as f32 / 15.0, 0.5]).collect();
        assert!(Curve::from_points(&sixteen).is_some());
    }
}
