//! The math node. It takes two driver signals and makes one, the way Blender's Math node does. A
//! dropdown picks the operation. The two inputs are knobs, each with a control pin. A wire into a
//! pin makes that input a driver signal, and an unwired input is the knob's own number, so the node
//! can also do math against a constant. Every signal runs from 0 to 1, and the result stays inside
//! that range.

use super::{Control, FlodeNode, KnobId, NodeBehavior, Port, Row};

pub struct Math;

/// What the node does to its two inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    /// A raised to a power that B sets: a quarter with B at 0, 1 with B at the middle, and 4 with
    /// B at the top.
    Power,
    Minimum,
    Maximum,
    /// How far apart A and B are.
    Difference,
    Average,
    /// 1 where A is above B, and 0 where it is not.
    GreaterThan,
    LessThan,
    /// What is left of A after taking away whole copies of B.
    Modulo,
}

impl MathOp {
    /// Every operation, in the order the dropdown lists them.
    pub const ALL: [MathOp; 12] = [
        MathOp::Add,
        MathOp::Subtract,
        MathOp::Multiply,
        MathOp::Divide,
        MathOp::Power,
        MathOp::Minimum,
        MathOp::Maximum,
        MathOp::Difference,
        MathOp::Average,
        MathOp::GreaterThan,
        MathOp::LessThan,
        MathOp::Modulo,
    ];

    /// The dropdown labels, in the order of `ALL`.
    pub const LABELS: [&'static str; 12] = [
        "Add",
        "Subtract",
        "Multiply",
        "Divide",
        "Power",
        "Minimum",
        "Maximum",
        "Difference",
        "Average",
        "Greater than",
        "Less than",
        "Modulo",
    ];

    /// The result for inputs `a` and `b`, which run from 0 to 1. It stops at 0 and 1, and an
    /// operation that has no answer, such as dividing by zero, gives 0.
    pub fn apply(self, a: f32, b: f32) -> f32 {
        // A divisor this small has no usable answer.
        const TINY: f32 = 1e-4;
        let result = match self {
            MathOp::Add => a + b,
            MathOp::Subtract => a - b,
            MathOp::Multiply => a * b,
            MathOp::Divide => {
                if b < TINY {
                    0.0
                } else {
                    a / b
                }
            }
            MathOp::Power => a.max(0.0).powf(4.0f32.powf(b * 2.0 - 1.0)),
            MathOp::Minimum => a.min(b),
            MathOp::Maximum => a.max(b),
            MathOp::Difference => (a - b).abs(),
            MathOp::Average => (a + b) / 2.0,
            MathOp::GreaterThan => f32::from(a > b),
            MathOp::LessThan => f32::from(a < b),
            MathOp::Modulo => {
                if b < TINY {
                    0.0
                } else {
                    a.rem_euclid(b)
                }
            }
        };
        if result.is_nan() {
            0.0
        } else {
            result.clamp(0.0, 1.0)
        }
    }
}

impl NodeBehavior for Math {
    fn title(&self) -> &'static str {
        "Math"
    }

    fn rows(&self, _node: &FlodeNode) -> Vec<Row> {
        vec![
            // The result. The output pin sits on a row of its own, with its name beside it.
            Row::default().with_output(Port::Shape),
            Row::control(Control::Dropdown(0)),
            Row::driven_knob(KnobId::MathA),
            Row::driven_knob(KnobId::MathB),
            // No pin: the span only sets what units A and B are shown in.
            Row::plain_knob(KnobId::MathSpan),
        ]
    }

    fn dropdowns(&self, _node: &FlodeNode) -> Vec<&'static [&'static str]> {
        vec![&MathOp::LABELS]
    }

    fn dropdown_names(&self) -> &'static [&'static str] {
        &["Op"]
    }
}

/// The operation a math node does.
pub fn operation(node: &FlodeNode) -> MathOp {
    MathOp::ALL[node.choice(0).min(MathOp::ALL.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;
    use nih_plug_iced::Point;

    fn node() -> FlodeNode {
        FlodeNode::new(NodeKind::Math, Point::new(0.0, 0.0))
    }

    fn near(actual: f32, expected: f32) -> bool {
        (actual - expected).abs() < 1e-5
    }

    #[test]
    fn the_labels_match_the_operations() {
        assert_eq!(MathOp::ALL.len(), MathOp::LABELS.len());
        let node = node();
        assert_eq!(
            node.kind.behavior().dropdowns(&node)[0].len(),
            MathOp::ALL.len()
        );
    }

    #[test]
    fn the_node_has_an_output_a_dropdown_and_two_pinned_knobs() {
        let node = node();
        let rows = node.rows();
        assert_eq!(node.output_ports(), vec![Port::Shape]);
        assert_eq!(
            node.input_ports(),
            vec![Port::Knob(KnobId::MathA), Port::Knob(KnobId::MathB)]
        );
        assert!(rows
            .iter()
            .any(|row| row.control == Some(Control::Dropdown(0))));
        assert_eq!(operation(&node), MathOp::Add);
    }

    #[test]
    fn the_dropdown_picks_the_operation() {
        let mut node = node();
        for (row, op) in MathOp::ALL.iter().enumerate() {
            node.choose(0, row);
            assert_eq!(operation(&node), *op);
        }
    }

    #[test]
    fn the_basic_operations() {
        assert!(near(MathOp::Add.apply(0.25, 0.5), 0.75));
        assert!(near(MathOp::Subtract.apply(0.75, 0.25), 0.5));
        assert!(near(MathOp::Multiply.apply(0.5, 0.5), 0.25));
        assert!(near(MathOp::Divide.apply(0.25, 0.5), 0.5));
        assert!(near(MathOp::Minimum.apply(0.25, 0.5), 0.25));
        assert!(near(MathOp::Maximum.apply(0.25, 0.5), 0.5));
        assert!(near(MathOp::Difference.apply(0.25, 0.75), 0.5));
        assert!(near(MathOp::Difference.apply(0.75, 0.25), 0.5));
        assert!(near(MathOp::Average.apply(0.25, 0.75), 0.5));
        assert!(near(MathOp::Modulo.apply(0.75, 0.5), 0.25));
    }

    /// A result that leaves 0 to 1 stops at the end.
    #[test]
    fn results_stay_between_zero_and_one() {
        assert_eq!(MathOp::Add.apply(0.75, 0.75), 1.0);
        assert_eq!(MathOp::Subtract.apply(0.25, 0.75), 0.0);
        assert_eq!(MathOp::Divide.apply(0.75, 0.25), 1.0);
    }

    #[test]
    fn dividing_by_zero_gives_zero() {
        assert_eq!(MathOp::Divide.apply(0.5, 0.0), 0.0);
        assert_eq!(MathOp::Modulo.apply(0.5, 0.0), 0.0);
    }

    #[test]
    fn the_comparisons_give_one_or_zero() {
        assert_eq!(MathOp::GreaterThan.apply(0.6, 0.4), 1.0);
        assert_eq!(MathOp::GreaterThan.apply(0.4, 0.6), 0.0);
        assert_eq!(MathOp::GreaterThan.apply(0.5, 0.5), 0.0);
        assert_eq!(MathOp::LessThan.apply(0.4, 0.6), 1.0);
        assert_eq!(MathOp::LessThan.apply(0.6, 0.4), 0.0);
    }

    /// B at the middle leaves A as it is. B at the top squares it twice, and B at the bottom takes
    /// its fourth root.
    #[test]
    fn power_runs_from_a_quarter_to_four() {
        assert!(near(MathOp::Power.apply(0.5, 0.5), 0.5));
        assert!(near(MathOp::Power.apply(0.5, 1.0), 0.0625));
        assert!(near(MathOp::Power.apply(0.0625, 0.0), 0.5));
        assert_eq!(MathOp::Power.apply(0.0, 0.7), 0.0);
        assert_eq!(MathOp::Power.apply(1.0, 0.2), 1.0);
    }

    /// At the default span A and B read as percents, as before. The span has no pin.
    #[test]
    fn the_default_span_reads_as_percents() {
        let mut node = node();
        node.set_value(KnobId::MathB, 0.5);
        assert_eq!(node.knob_format(KnobId::MathB, 0.5), "50%");
        assert_eq!(node.knob_entry_text(KnobId::MathB), "50");
        let span = Math
            .rows(&node)
            .into_iter()
            .find(|row| row.knob == Some(KnobId::MathSpan))
            .expect("the span row");
        assert!(span.input.is_none());
    }

    /// With a span of 24, typing 5 into B is 5 of 24 of the signal, and B reads back as 5.
    #[test]
    fn a_span_gives_a_and_b_units() {
        let mut node = node();
        node.enter_knob_value(KnobId::MathSpan, 24.0);
        assert!((node.plain(KnobId::MathSpan) - 24.0).abs() < 1e-3);
        node.enter_knob_value(KnobId::MathB, 5.0);
        assert!((node.value(KnobId::MathB) - 5.0 / 24.0).abs() < 1e-4);
        assert_eq!(node.knob_entry_text(KnobId::MathB), "5");
        assert_eq!(
            node.knob_format(KnobId::MathB, node.value(KnobId::MathB)),
            "5"
        );
        // Adding it to a signal moves the signal by 5 of its 24.
        assert!(near(
            MathOp::Add.apply(0.5, node.value(KnobId::MathB)),
            0.5 + 5.0 / 24.0
        ));
    }

    /// A number past the span stops at the span.
    #[test]
    fn a_number_past_the_span_stops_at_it() {
        let mut node = node();
        node.enter_knob_value(KnobId::MathSpan, 24.0);
        node.enter_knob_value(KnobId::MathA, 100.0);
        assert_eq!(node.value(KnobId::MathA), 1.0);
    }

    /// Every operation stays finite and inside 0 to 1 for inputs at and past the ends.
    #[test]
    fn every_operation_is_safe_at_the_extremes() {
        let inputs = [-1.0, 0.0, 1e-6, 0.5, 1.0, 2.0, f32::NAN];
        for op in MathOp::ALL {
            for a in inputs {
                for b in inputs {
                    let result = op.apply(a, b);
                    assert!((0.0..=1.0).contains(&result), "{op:?}({a}, {b}) = {result}");
                }
            }
        }
    }
}
