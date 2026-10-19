//! The monitor node's screen, and the history the time based modes keep. The audio thread reports
//! one measurement per monitor instance. The spectrogram and the level strip need the last few
//! seconds of those, so the editor keeps them here, in rings that are allocated once.

use crate::dsp::analyzer::{BINS, HIGH_HZ, LOW_HZ};
use crate::dsp::monitor::{MonitorData, MonitorMode, TRACE_POINTS};
use crate::nodes::monitor;
use nih_plug_iced::widget::canvas::{Frame, LineCap, Path, Stroke, Text};
use nih_plug_iced::{alignment, Color, Point, Rectangle, Size};
use std::time::{Duration, Instant};

use super::draw::{draw_text, rounded_rect, smooth_segments};
use super::graph::Graph;
use super::style::*;

/// Columns of the spectrogram, and samples of the level strip, that a monitor keeps.
pub(super) const SPECTROGRAM_COLUMNS: usize = 48;
pub(super) const LEVEL_SAMPLES: usize = 160;
/// How often a history takes a new column.
const PUSH_INTERVAL: Duration = Duration::from_millis(33);
/// How long the clip light stays lit after a peak reaches `CLIP_DB`.
const CLIP_HOLD: Duration = Duration::from_secs(1);
pub(super) const CLIP_DB: f32 = -0.1;
/// The bottom of the level bars and strip, and the span of the spectrum and spectrogram, in dB.
const LEVEL_FLOOR_DB: f32 = -60.0;
const SPECTRUM_SPAN_DB: f32 = 96.0;
const SPECTROGRAM_SPAN_DB: f32 = 90.0;
/// How many colors the spectrogram uses. Fewer levels merge into longer runs.
const SPECTROGRAM_LEVELS: usize = 12;

pub(super) const NO_SIGNAL_HINT: &str = "Wire it into a chain that reaches the output";

/// A ring of the last `N` values. Nothing is allocated after it is built.
pub(super) struct Ring<T: Copy, const N: usize> {
    items: [T; N],
    /// Where the next value goes.
    head: usize,
    len: usize,
}

impl<T: Copy, const N: usize> Ring<T, N> {
    pub(super) fn new(fill: T) -> Self {
        Self {
            items: [fill; N],
            head: 0,
            len: 0,
        }
    }

    pub(super) fn push(&mut self, value: T) {
        self.items[self.head] = value;
        self.head = (self.head + 1) % N;
        self.len = (self.len + 1).min(N);
    }

    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    /// The value `age` steps from the oldest one kept, so 0 is the oldest and `len() - 1` the
    /// newest.
    pub(super) fn get(&self, age: usize) -> Option<T> {
        (age < self.len).then(|| self.items[(self.head + N - self.len + age) % N])
    }
}

/// What one monitor node remembers between frames.
pub(super) struct MonitorHistory {
    pub(super) node: usize,
    mode: MonitorMode,
    /// The spectrum in dB at each column, for the spectrogram.
    pub(super) spectrogram: Ring<[f32; BINS], SPECTROGRAM_COLUMNS>,
    /// The loudest peak and RMS of the two channels in dB, for the level strip.
    pub(super) levels: Ring<[f32; 2], LEVEL_SAMPLES>,
    last_push: Option<Instant>,
    clip_until: Option<Instant>,
    /// True while the clip light is lit.
    pub(super) clip: bool,
}

impl MonitorHistory {
    pub(super) fn new(node: usize, mode: MonitorMode) -> Self {
        Self {
            node,
            mode,
            spectrogram: Ring::new([0.0; BINS]),
            levels: Ring::new([0.0; 2]),
            last_push: None,
            clip_until: None,
            clip: false,
        }
    }

    /// Takes in one frame of a monitor in `mode`. Changing mode starts the history over. With no
    /// data nothing is added, and the clip light still runs out.
    pub(super) fn feed(&mut self, mode: MonitorMode, data: Option<&MonitorData>, now: Instant) {
        if mode != self.mode {
            self.mode = mode;
            self.spectrogram.clear();
            self.levels.clear();
            self.last_push = None;
            self.clip_until = None;
        }
        if let Some(data) = data {
            if mode == MonitorMode::Levels && peak_db(data) >= CLIP_DB {
                self.clip_until = Some(now + CLIP_HOLD);
            }
            if push_due(self.last_push, now) {
                match mode {
                    MonitorMode::Spectrogram => self.spectrogram.push(data.spectrum),
                    MonitorMode::Levels => self
                        .levels
                        .push([peak_db(data), data.levels[2].max(data.levels[3])]),
                    _ => {}
                }
                self.last_push = Some(now);
            }
        }
        self.clip = self.clip_until.is_some_and(|until| now < until);
    }
}

/// The loudest peak of the two channels, in dB.
fn peak_db(data: &MonitorData) -> f32 {
    data.levels[0].max(data.levels[1])
}

/// True if a new column is due.
pub(super) fn push_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|last| now.duration_since(last) >= PUSH_INTERVAL)
}

/// The line a screen shows in place of its picture, if there is no measurement to draw.
pub(super) fn hint(has_data: bool) -> Option<&'static str> {
    (!has_data).then_some(NO_SIGNAL_HINT)
}

/// A level in dB as a position from 0.0 at `-span` to 1.0 at 0 dB.
pub(super) fn db_fraction(db: f32, span: f32) -> f32 {
    ((db + span) / span).clamp(0.0, 1.0)
}

/// A level in dB as a position along a level bar, from 0.0 at -60 dB to 1.0 at 0 dB.
pub(super) fn bar_position(db: f32) -> f32 {
    db_fraction(db, -LEVEL_FLOOR_DB)
}

/// A frequency as a position from 0.0 at 20 Hz to 1.0 at 20 kHz, on the log axis.
pub(super) fn frequency_position(freq: f32) -> f32 {
    ((freq / LOW_HZ).ln() / (HIGH_HZ / LOW_HZ).ln()).clamp(0.0, 1.0)
}

/// Where a left and right sample plot on a goniometer, as x to the right and y up, each from -1.0
/// to 1.0. Left alone goes to the upper left and right alone to the upper right, and equal
/// samples go straight up the middle. Every point lands inside the diamond.
pub(super) fn goniometer(left: f32, right: f32) -> (f32, f32) {
    let (l, r) = (left.clamp(-1.0, 1.0), right.clamp(-1.0, 1.0));
    ((r - l) / 2.0, (l + r) / 2.0)
}

/// A correlation from -1.0 to 1.0 as a position along its bar, from 0.0 to 1.0.
pub(super) fn correlation_position(correlation: f32) -> f32 {
    ((correlation + 1.0) / 2.0).clamp(0.0, 1.0)
}

/// The spectrogram color ramp, from dark blue through purple and orange to near white. `t` runs
/// from 0.0 to 1.0.
pub(super) fn ramp_color(t: f32) -> Color {
    const STOPS: [[f32; 3]; 5] = [
        [0.02, 0.02, 0.13],
        [0.18, 0.08, 0.42],
        [0.55, 0.14, 0.50],
        [0.95, 0.52, 0.12],
        [1.0, 0.97, 0.86],
    ];
    let scaled = t.clamp(0.0, 1.0) * (STOPS.len() - 1) as f32;
    let i = (scaled as usize).min(STOPS.len() - 2);
    let f = scaled - i as f32;
    let mix = |c: usize| STOPS[i][c] + (STOPS[i + 1][c] - STOPS[i][c]) * f;
    Color::from_rgb(mix(0), mix(1), mix(2))
}

/// The color level of `t`, from 0 to `SPECTROGRAM_LEVELS - 1`.
pub(super) fn quantize(t: f32) -> usize {
    ((t.clamp(0.0, 1.0) * SPECTROGRAM_LEVELS as f32) as usize).min(SPECTROGRAM_LEVELS - 1)
}

/// The color a quantized level draws in.
pub(super) fn level_color(level: usize) -> Color {
    ramp_color(level as f32 / (SPECTROGRAM_LEVELS - 1) as f32)
}

/// Calls `f(start, length, level)` for each run of equal, nonzero levels. Level 0 is the screen's
/// own color, so it needs no shape.
pub(super) fn for_runs(
    levels: impl Iterator<Item = usize>,
    mut f: impl FnMut(usize, usize, usize),
) {
    let mut run: Option<(usize, usize, usize)> = None;
    for (i, level) in levels.enumerate() {
        match run {
            Some((start, length, current)) if current == level => {
                run = Some((start, length + 1, level))
            }
            _ => {
                if let Some((start, length, current)) = run {
                    if current > 0 {
                        f(start, length, current);
                    }
                }
                run = Some((i, 1, level));
            }
        }
    }
    if let Some((start, length, current)) = run {
        if current > 0 {
            f(start, length, current);
        }
    }
}

/// The measurement of the instance that `pools` gives `node`, if it has one.
pub(super) fn instance_of<'a>(
    pools: &[(usize, u8)],
    monitors: &'a [MonitorData],
    node: usize,
) -> Option<&'a MonitorData> {
    let (_, pool) = pools.iter().find(|(found, _)| *found == node)?;
    monitors.get(usize::from(*pool))
}

impl Graph {
    /// What a monitor node's instance measured, or None if the node listens to nothing that
    /// reaches an output slot.
    pub(super) fn monitor_data(&self, node: usize) -> Option<&MonitorData> {
        instance_of(&self.wiring().monitor_pools(), &self.monitors, node)
    }

    pub(super) fn monitor_history_of(&self, node: usize) -> Option<&MonitorHistory> {
        self.monitor_history.iter().find(|h| h.node == node)
    }

    /// Adds this frame's measurements to the histories of the monitors that need one. Called once
    /// per frame, with the measurements already copied in.
    pub(super) fn update_monitors(&mut self, now: Instant) {
        if !self
            .nodes
            .iter()
            .any(|node| node.kind == crate::nodes::NodeKind::Monitor)
        {
            return;
        }
        let pools = self.wiring().monitor_pools();
        for index in 0..self.nodes.len() {
            if self.nodes[index].kind != crate::nodes::NodeKind::Monitor {
                continue;
            }
            let mode = monitor::mode(&self.nodes[index]);
            let data = instance_of(&pools, &self.monitors, index);
            let slot = match self.monitor_history.iter().position(|h| h.node == index) {
                Some(slot) => slot,
                None if matches!(mode, MonitorMode::Spectrogram | MonitorMode::Levels) => {
                    self.monitor_history.push(MonitorHistory::new(index, mode));
                    self.monitor_history.len() - 1
                }
                None => continue,
            };
            self.monitor_history[slot].feed(mode, data, now);
        }
    }

    /// Forgets the history of a deleted node, and renumbers the ones after it.
    pub(super) fn drop_monitor_history(&mut self, index: usize) {
        self.monitor_history.retain(|h| h.node != index);
        for history in &mut self.monitor_history {
            if history.node > index {
                history.node -= 1;
            }
        }
    }
}

/// Draws the screen of the monitor node `index`, if it has one on show.
pub(super) fn draw_monitor(
    frame: &mut Frame,
    index: usize,
    graph: &Graph,
    covered: &[Rectangle],
    zoom: f32,
) {
    let node = &graph.nodes[index];
    let Some(bounds) = node.monitor_bounds() else {
        return;
    };
    draw_screen(
        frame,
        bounds,
        monitor::mode(node),
        graph.monitor_data(index),
        graph.monitor_history_of(index),
        covered,
        zoom,
    );
}

fn faint(alpha: f32) -> Stroke<'static> {
    Stroke::default()
        .with_color(Color {
            a: alpha,
            ..TEXT_COLOR
        })
        .with_width(1.0)
}

fn line(frame: &mut Frame, a: Point, b: Point, alpha: f32) {
    frame.stroke(&Path::line(a, b), faint(alpha));
}

fn rect(frame: &mut Frame, x: f32, y: f32, width: f32, height: f32, color: Color) {
    if width > 0.0 && height > 0.0 {
        frame.fill_rectangle(Point::new(x, y), Size::new(width, height), color);
    }
}

struct Labels<'a> {
    covered: &'a [Rectangle],
    zoom: f32,
}

impl Labels<'_> {
    fn put(
        &self,
        frame: &mut Frame,
        content: &str,
        at: Point,
        horizontal: alignment::Horizontal,
        size: f32,
    ) {
        draw_text(
            frame,
            self.covered,
            self.zoom,
            Text {
                content: content.to_string(),
                position: at,
                color: DIM_TEXT_COLOR,
                size,
                horizontal_alignment: horizontal,
                vertical_alignment: alignment::Vertical::Center,
                ..Text::default()
            },
        );
    }
}

/// The screen: a dark panel with the picture of `mode` on it, or the hint if there is no `data`.
pub(super) fn draw_screen(
    frame: &mut Frame,
    bounds: Rectangle,
    mode: MonitorMode,
    data: Option<&MonitorData>,
    history: Option<&MonitorHistory>,
    covered: &[Rectangle],
    zoom: f32,
) {
    frame.fill(
        &rounded_rect(bounds, DROPDOWN_CORNER_RADIUS),
        MONITOR_SCREEN_COLOR,
    );
    let labels = Labels { covered, zoom };
    if let Some(text) = hint(data.is_some()) {
        labels.put(
            frame,
            text,
            Point::new(
                bounds.x + bounds.width / 2.0,
                bounds.y + bounds.height / 2.0,
            ),
            alignment::Horizontal::Center,
            10.0,
        );
        return;
    }
    let Some(data) = data else {
        return;
    };
    match mode {
        MonitorMode::Scope => draw_scope(frame, bounds, data),
        MonitorMode::Spectrum => draw_spectrum(frame, bounds, data, &labels),
        MonitorMode::Spectrogram => draw_spectrogram(frame, bounds, history, &labels),
        MonitorMode::Levels => draw_levels(frame, bounds, data, history, &labels),
        MonitorMode::Vectorscope => draw_vectorscope(frame, bounds, data, &labels),
    }
}

fn inset(bounds: Rectangle, left: f32, top: f32, right: f32, bottom: f32) -> Rectangle {
    Rectangle {
        x: bounds.x + left,
        y: bounds.y + top,
        width: bounds.width - left - right,
        height: bounds.height - top - bottom,
    }
}

fn draw_scope(frame: &mut Frame, bounds: Rectangle, data: &MonitorData) {
    let plot = inset(bounds, 6.0, 8.0, 6.0, 8.0);
    let mid = plot.y + plot.height / 2.0;
    for i in 1..4 {
        let x = plot.x + plot.width * i as f32 / 4.0;
        line(
            frame,
            Point::new(x, plot.y),
            Point::new(x, plot.y + plot.height),
            0.06,
        );
    }
    for fraction in [0.25, 0.75] {
        let y = plot.y + plot.height * fraction;
        line(
            frame,
            Point::new(plot.x, y),
            Point::new(plot.x + plot.width, y),
            0.06,
        );
    }
    line(
        frame,
        Point::new(plot.x, mid),
        Point::new(plot.x + plot.width, mid),
        0.22,
    );

    let same = data.trace.iter().all(|sample| sample[0] == sample[1]);
    let channels: &[(usize, Color)] = if same {
        &[(0, MONITOR_LEFT_COLOR)]
    } else {
        &[(0, MONITOR_LEFT_COLOR), (1, MONITOR_RIGHT_COLOR)]
    };
    for &(channel, color) in channels {
        let points: Vec<Point> = (0..TRACE_POINTS)
            .map(|i| {
                let value = data.trace[i][channel].clamp(-1.0, 1.0);
                Point::new(
                    plot.x + plot.width * i as f32 / (TRACE_POINTS - 1) as f32,
                    mid - value * plot.height / 2.0,
                )
            })
            .collect();
        let path = Path::new(|builder| {
            builder.move_to(points[0]);
            for (c1, c2, end) in smooth_segments(&points, plot.y, plot.y + plot.height) {
                builder.bezier_curve_to(c1, c2, end);
            }
        });
        frame.stroke(
            &path,
            Stroke::default()
                .with_color(color)
                .with_width(1.5)
                .with_line_cap(LineCap::Round),
        );
    }
}

fn draw_spectrum(frame: &mut Frame, bounds: Rectangle, data: &MonitorData, labels: &Labels) {
    let plot = inset(bounds, 26.0, 8.0, 8.0, 16.0);
    let bottom = plot.y + plot.height;
    for db in [0.0, -24.0, -48.0, -72.0, -96.0] {
        let y = plot.y + (1.0 - db_fraction(db, SPECTRUM_SPAN_DB)) * plot.height;
        line(
            frame,
            Point::new(plot.x, y),
            Point::new(plot.x + plot.width, y),
            0.08,
        );
        let text = format!("{db:.0}");
        labels.put(
            frame,
            &text,
            Point::new(bounds.x + 4.0, y.clamp(plot.y + 4.0, bottom - 4.0)),
            alignment::Horizontal::Left,
            8.0,
        );
    }
    for (freq, text) in [(100.0, "100"), (1000.0, "1k"), (10000.0, "10k")] {
        let x = plot.x + frequency_position(freq) * plot.width;
        line(frame, Point::new(x, plot.y), Point::new(x, bottom), 0.08);
        labels.put(
            frame,
            text,
            Point::new(x, bottom + 8.0),
            alignment::Horizontal::Center,
            8.0,
        );
    }
    let points: Vec<Point> = data
        .spectrum
        .iter()
        .enumerate()
        .map(|(i, db)| {
            Point::new(
                plot.x + (i as f32 + 0.5) / BINS as f32 * plot.width,
                plot.y + (1.0 - db_fraction(*db, SPECTRUM_SPAN_DB)) * plot.height,
            )
        })
        .collect();
    let curve = |builder: &mut nih_plug_iced::widget::canvas::path::Builder| {
        for (c1, c2, end) in smooth_segments(&points, plot.y, bottom) {
            builder.bezier_curve_to(c1, c2, end);
        }
    };
    let area = Path::new(|builder| {
        builder.move_to(Point::new(points[0].x, bottom));
        builder.line_to(points[0]);
        curve(builder);
        builder.line_to(Point::new(points[BINS - 1].x, bottom));
        builder.close();
    });
    frame.fill(&area, ANALYZER_COLOR);
    let top = Path::new(|builder| {
        builder.move_to(points[0]);
        curve(builder);
    });
    frame.stroke(
        &top,
        Stroke::default()
            .with_color(SELECTION_COLOR)
            .with_width(1.5)
            .with_line_cap(LineCap::Round),
    );
}

fn draw_spectrogram(
    frame: &mut Frame,
    bounds: Rectangle,
    history: Option<&MonitorHistory>,
    labels: &Labels,
) {
    let plot = inset(bounds, 22.0, 6.0, 6.0, 6.0);
    rect(
        frame,
        plot.x,
        plot.y,
        plot.width,
        plot.height,
        level_color(0),
    );
    let column = plot.width / SPECTROGRAM_COLUMNS as f32;
    let row = plot.height / BINS as f32;
    if let Some(history) = history {
        let ring = &history.spectrogram;
        let first = SPECTROGRAM_COLUMNS - ring.len();
        for bin in 0..BINS {
            // Low bands sit at the bottom.
            let y = plot.y + plot.height - (bin + 1) as f32 * row;
            let levels = (0..ring.len()).map(|age| {
                let db = ring.get(age).map_or(f32::MIN, |spectrum| spectrum[bin]);
                quantize(db_fraction(db, SPECTROGRAM_SPAN_DB))
            });
            for_runs(levels, |start, length, level| {
                rect(
                    frame,
                    plot.x + (first + start) as f32 * column,
                    y,
                    length as f32 * column + 0.5,
                    row + 0.5,
                    level_color(level),
                );
            });
        }
    }
    for (freq, text) in [(100.0, "100"), (1000.0, "1k"), (10000.0, "10k")] {
        let y = plot.y + plot.height - frequency_position(freq) * plot.height;
        line(
            frame,
            Point::new(plot.x, y),
            Point::new(plot.x + plot.width, y),
            0.12,
        );
        labels.put(
            frame,
            text,
            Point::new(bounds.x + 4.0, y),
            alignment::Horizontal::Left,
            8.0,
        );
    }
}

/// The color of the level bar from `from` to `to` dB, in the zones of green, yellow and red.
const ZONES: [(f32, f32, Color); 3] = [
    (-60.0, -18.0, LEVEL_GREEN_COLOR),
    (-18.0, -6.0, LEVEL_YELLOW_COLOR),
    (-6.0, 0.0, LEVEL_RED_COLOR),
];

fn draw_levels(
    frame: &mut Frame,
    bounds: Rectangle,
    data: &MonitorData,
    history: Option<&MonitorHistory>,
    labels: &Labels,
) {
    let bars = Rectangle {
        x: bounds.x + 16.0,
        y: bounds.y + 8.0,
        width: bounds.width - 16.0 - 22.0,
        height: 12.0,
    };
    for channel in 0..2 {
        let y = bars.y + channel as f32 * (bars.height + 4.0);
        labels.put(
            frame,
            if channel == 0 { "L" } else { "R" },
            Point::new(bounds.x + 6.0, y + bars.height / 2.0),
            alignment::Horizontal::Left,
            9.0,
        );
        let rms = bar_position(data.levels[2 + channel]);
        for (from, to, color) in ZONES {
            let (a, b) = (bar_position(from), bar_position(to));
            rect(
                frame,
                bars.x + a * bars.width,
                y,
                (b - a) * bars.width,
                bars.height,
                Color { a: 0.18, ..color },
            );
            let filled = rms.min(b) - a;
            rect(
                frame,
                bars.x + a * bars.width,
                y,
                filled * bars.width,
                bars.height,
                color,
            );
        }
        let peak = bars.x + bar_position(data.levels[channel]) * bars.width;
        rect(
            frame,
            peak - 1.0,
            y - 1.0,
            2.0,
            bars.height + 2.0,
            TEXT_COLOR,
        );
    }
    let lit = history.is_some_and(|history| history.clip);
    let light = Point::new(bounds.x + bounds.width - 11.0, bars.y + bars.height + 2.0);
    frame.fill(
        &Path::circle(light, 4.0),
        if lit {
            LEVEL_RED_COLOR
        } else {
            Color {
                a: 0.2,
                ..LEVEL_RED_COLOR
            }
        },
    );

    let scale_y = bars.y + 2.0 * bars.height + 4.0 + 8.0;
    for db in [-60.0, -48.0, -36.0, -24.0, -12.0, 0.0] {
        let x = bars.x + bar_position(db) * bars.width;
        line(
            frame,
            Point::new(x, bars.y),
            Point::new(x, bars.y + 2.0 * bars.height + 4.0),
            0.06,
        );
        let (text, align) = (format!("{db:.0}"), alignment::Horizontal::Center);
        labels.put(frame, &text, Point::new(x, scale_y), align, 8.0);
    }

    let strip = Rectangle {
        x: bounds.x + 6.0,
        y: scale_y + 10.0,
        width: bounds.width - 12.0,
        height: bounds.y + bounds.height - 6.0 - (scale_y + 10.0),
    };
    frame.fill(
        &rounded_rect(strip, 3.0),
        Color {
            a: 0.35,
            ..Color::BLACK
        },
    );
    for db in [-48.0, -36.0, -24.0, -12.0] {
        let y = strip.y + (1.0 - bar_position(db)) * strip.height;
        line(
            frame,
            Point::new(strip.x, y),
            Point::new(strip.x + strip.width, y),
            0.05,
        );
    }
    let Some(history) = history else {
        return;
    };
    let ring = &history.levels;
    if ring.len() < 2 {
        return;
    }
    let first = LEVEL_SAMPLES - ring.len();
    let at = |age: usize, db: f32| {
        Point::new(
            strip.x + (first + age) as f32 / (LEVEL_SAMPLES - 1) as f32 * strip.width,
            strip.y + (1.0 - bar_position(db)) * strip.height,
        )
    };
    let bottom = strip.y + strip.height;
    let area = Path::new(|builder| {
        builder.move_to(Point::new(at(0, 0.0).x, bottom));
        for age in 0..ring.len() {
            builder.line_to(at(age, ring.get(age).map_or(f32::MIN, |s| s[1])));
        }
        builder.line_to(Point::new(at(ring.len() - 1, 0.0).x, bottom));
        builder.close();
    });
    frame.fill(
        &area,
        Color {
            a: 0.55,
            ..LEVEL_GREEN_COLOR
        },
    );
    let peaks = Path::new(|builder| {
        for age in 0..ring.len() {
            let point = at(age, ring.get(age).map_or(f32::MIN, |s| s[0]));
            if age == 0 {
                builder.move_to(point);
            } else {
                builder.line_to(point);
            }
        }
    });
    frame.stroke(&peaks, faint(0.8));
}

fn draw_vectorscope(frame: &mut Frame, bounds: Rectangle, data: &MonitorData, labels: &Labels) {
    let half = (bounds.height - 30.0) / 2.0;
    let center = Point::new(bounds.x + bounds.width / 2.0, bounds.y + 4.0 + half);
    let at = |x: f32, y: f32| Point::new(center.x + x * half, center.y - y * half);
    let diamond = Path::new(|builder| {
        builder.move_to(at(0.0, 1.0));
        builder.line_to(at(1.0, 0.0));
        builder.line_to(at(0.0, -1.0));
        builder.line_to(at(-1.0, 0.0));
        builder.close();
    });
    frame.stroke(&diamond, faint(0.18));
    line(frame, at(0.0, 1.0), at(0.0, -1.0), 0.1);
    line(frame, at(-1.0, 0.0), at(1.0, 0.0), 0.1);
    line(frame, at(-0.5, 0.5), at(0.5, -0.5), 0.1);
    line(frame, at(0.5, 0.5), at(-0.5, -0.5), 0.1);
    for (text, x) in [("L", -0.5), ("R", 0.5)] {
        let p = at(x * 1.25, 0.5 * 1.25);
        labels.put(frame, text, p, alignment::Horizontal::Center, 9.0);
    }
    for sample in &data.trace {
        let (x, y) = goniometer(sample[0], sample[1]);
        let p = at(x, y);
        rect(frame, p.x - 1.0, p.y - 1.0, 2.0, 2.0, VECTORSCOPE_DOT_COLOR);
    }

    let bar = Rectangle {
        x: bounds.x + 22.0,
        y: bounds.y + bounds.height - 14.0,
        width: bounds.width - 22.0 - 70.0,
        height: 6.0,
    };
    let mid = bar.x + bar.width / 2.0;
    rect(
        frame,
        bar.x,
        bar.y,
        bar.width,
        bar.height,
        Color {
            a: 0.12,
            ..TEXT_COLOR
        },
    );
    let marker = bar.x + correlation_position(data.correlation) * bar.width;
    let (from, to) = (mid.min(marker), mid.max(marker));
    let color = if data.correlation >= 0.0 {
        LEVEL_GREEN_COLOR
    } else {
        LEVEL_RED_COLOR
    };
    rect(
        frame,
        from,
        bar.y,
        to - from,
        bar.height,
        Color { a: 0.6, ..color },
    );
    rect(
        frame,
        mid - 0.5,
        bar.y - 2.0,
        1.0,
        bar.height + 4.0,
        Color {
            a: 0.4,
            ..TEXT_COLOR
        },
    );
    rect(
        frame,
        marker - 1.0,
        bar.y - 2.0,
        2.0,
        bar.height + 4.0,
        TEXT_COLOR,
    );
    let y = bar.y + bar.height / 2.0;
    labels.put(
        frame,
        "-1",
        Point::new(bounds.x + 6.0, y),
        alignment::Horizontal::Left,
        8.0,
    );
    labels.put(
        frame,
        "+1",
        Point::new(bar.x + bar.width + 4.0, y),
        alignment::Horizontal::Left,
        8.0,
    );
    let number = format!("{:+.2}", data.correlation);
    labels.put(
        frame,
        &number,
        Point::new(bounds.x + bounds.width - 6.0, y),
        alignment::Horizontal::Right,
        10.0,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ring_fills_then_drops_the_oldest() {
        let mut ring: Ring<u32, 4> = Ring::new(0);
        assert_eq!((ring.len(), ring.get(0)), (0, None));
        for value in 1..=3 {
            ring.push(value);
        }
        assert_eq!(
            (ring.len(), ring.get(0), ring.get(2)),
            (3, Some(1), Some(3))
        );
        for value in 4..=6 {
            ring.push(value);
        }
        let kept: Vec<u32> = (0..ring.len()).filter_map(|i| ring.get(i)).collect();
        assert_eq!(kept, vec![3, 4, 5, 6]);
        assert_eq!(ring.get(4), None);
        ring.clear();
        assert_eq!(ring.len(), 0);
    }

    #[test]
    fn a_column_is_due_every_third_of_a_frame_interval() {
        let start = Instant::now();
        assert!(push_due(None, start));
        assert!(!push_due(Some(start), start + Duration::from_millis(10)));
        assert!(push_due(Some(start), start + Duration::from_millis(33)));
    }

    fn spectrum_data() -> MonitorData {
        MonitorData {
            spectrum: [-20.0; BINS],
            levels: [-3.0, -6.0, -12.0, -15.0],
            ..MonitorData::default()
        }
    }

    #[test]
    fn history_fills_only_in_its_own_mode_and_with_data() {
        let start = Instant::now();
        let data = spectrum_data();
        let mut history = MonitorHistory::new(0, MonitorMode::Spectrogram);
        history.feed(MonitorMode::Spectrogram, None, start);
        assert_eq!(history.spectrogram.len(), 0);
        history.feed(MonitorMode::Spectrogram, Some(&data), start);
        assert_eq!(history.spectrogram.len(), 1);
        assert_eq!(history.levels.len(), 0);
        // A frame sooner than the interval adds nothing.
        history.feed(
            MonitorMode::Spectrogram,
            Some(&data),
            start + Duration::from_millis(5),
        );
        assert_eq!(history.spectrogram.len(), 1);
        for i in 1..(SPECTROGRAM_COLUMNS as u64 + 10) {
            let now = start + Duration::from_millis(40 * i);
            history.feed(MonitorMode::Spectrogram, Some(&data), now);
        }
        assert_eq!(history.spectrogram.len(), SPECTROGRAM_COLUMNS);
        // Changing mode starts over.
        history.feed(
            MonitorMode::Levels,
            Some(&data),
            start + Duration::from_secs(10),
        );
        assert_eq!(history.spectrogram.len(), 0);
        assert_eq!(history.levels.get(0), Some([-3.0, -12.0]));
    }

    #[test]
    fn the_level_ring_wraps() {
        let start = Instant::now();
        let data = spectrum_data();
        let mut history = MonitorHistory::new(0, MonitorMode::Levels);
        for i in 0..(LEVEL_SAMPLES as u64 * 2) {
            history.feed(
                MonitorMode::Levels,
                Some(&data),
                start + Duration::from_millis(40 * i),
            );
        }
        assert_eq!(history.levels.len(), LEVEL_SAMPLES);
    }

    #[test]
    fn the_clip_light_holds_for_a_second() {
        let start = Instant::now();
        let mut data = spectrum_data();
        let mut history = MonitorHistory::new(0, MonitorMode::Levels);
        history.feed(MonitorMode::Levels, Some(&data), start);
        assert!(!history.clip);
        data.levels[1] = -0.1;
        history.feed(
            MonitorMode::Levels,
            Some(&data),
            start + Duration::from_millis(100),
        );
        assert!(history.clip);
        data.levels[1] = -20.0;
        history.feed(
            MonitorMode::Levels,
            Some(&data),
            start + Duration::from_millis(900),
        );
        assert!(history.clip);
        history.feed(
            MonitorMode::Levels,
            Some(&data),
            start + Duration::from_millis(1200),
        );
        assert!(!history.clip);
    }

    #[test]
    fn a_screen_without_data_shows_the_hint() {
        assert_eq!(hint(false), Some(NO_SIGNAL_HINT));
        assert_eq!(hint(true), None);
    }

    #[test]
    fn an_instance_is_found_by_its_node() {
        let mut monitors = [MonitorData::default(); 4];
        monitors[1].correlation = 0.25;
        monitors[3].correlation = -0.5;
        let pools = [(5, 3), (2, 1)];
        assert_eq!(
            instance_of(&pools, &monitors, 2).map(|m| m.correlation),
            Some(0.25)
        );
        assert_eq!(
            instance_of(&pools, &monitors, 5).map(|m| m.correlation),
            Some(-0.5)
        );
        assert!(instance_of(&pools, &monitors, 0).is_none());
        // A pool number past the end finds nothing.
        assert!(instance_of(&[(0, 9)], &monitors, 0).is_none());
    }

    #[test]
    fn levels_map_onto_the_bar() {
        assert_eq!(bar_position(-60.0), 0.0);
        assert_eq!(bar_position(0.0), 1.0);
        assert_eq!(bar_position(-30.0), 0.5);
        assert_eq!(bar_position(-100.0), 0.0);
        assert_eq!(bar_position(6.0), 1.0);
    }

    #[test]
    fn frequencies_map_onto_a_log_axis() {
        assert_eq!(frequency_position(20.0), 0.0);
        assert!((frequency_position(20000.0) - 1.0).abs() < 1e-6);
        assert!((frequency_position(632.4555) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn the_goniometer_puts_left_and_right_on_the_diagonals() {
        assert_eq!(goniometer(1.0, 1.0), (0.0, 1.0));
        assert_eq!(goniometer(1.0, 0.0), (-0.5, 0.5));
        assert_eq!(goniometer(0.0, 1.0), (0.5, 0.5));
        assert_eq!(goniometer(1.0, -1.0), (-1.0, 0.0));
        assert_eq!(goniometer(5.0, 5.0), (0.0, 1.0));
        for (l, r) in [(0.3, -0.9), (-1.0, -1.0), (0.7, 0.2)] {
            let (x, y) = goniometer(l, r);
            assert!(x.abs() + y.abs() <= 1.0 + 1e-6);
        }
    }

    #[test]
    fn the_correlation_bar_runs_from_minus_one_to_one() {
        assert_eq!(correlation_position(-1.0), 0.0);
        assert_eq!(correlation_position(0.0), 0.5);
        assert_eq!(correlation_position(1.0), 1.0);
        assert_eq!(correlation_position(3.0), 1.0);
    }

    #[test]
    fn the_color_ramp_brightens_from_dark_blue_to_near_white() {
        let (dark, light) = (ramp_color(0.0), ramp_color(1.0));
        assert!(dark.b > dark.r && dark.b < 0.2);
        assert!(light.r > 0.95 && light.g > 0.9 && light.b > 0.8);
        let luma = |c: Color| c.r * 0.3 + c.g * 0.6 + c.b * 0.1;
        let steps: Vec<f32> = (0..SPECTROGRAM_LEVELS)
            .map(|l| luma(level_color(l)))
            .collect();
        assert!(steps.windows(2).all(|pair| pair[0] < pair[1]), "{steps:?}");
    }

    #[test]
    fn quantizing_covers_every_level_once() {
        assert_eq!(quantize(0.0), 0);
        assert_eq!(quantize(1.0), SPECTROGRAM_LEVELS - 1);
        assert_eq!(quantize(-3.0), 0);
        assert_eq!(quantize(9.0), SPECTROGRAM_LEVELS - 1);
        let seen: std::collections::HashSet<usize> =
            (0..=1000).map(|i| quantize(i as f32 / 1000.0)).collect();
        assert_eq!(seen.len(), SPECTROGRAM_LEVELS);
    }

    fn runs(levels: &[usize]) -> Vec<(usize, usize, usize)> {
        let mut found = Vec::new();
        for_runs(levels.iter().copied(), |start, length, level| {
            found.push((start, length, level))
        });
        found
    }

    #[test]
    fn equal_neighbors_merge_into_one_run_and_the_darkest_level_is_skipped() {
        assert_eq!(runs(&[]), vec![]);
        assert_eq!(runs(&[0, 0, 0]), vec![]);
        assert_eq!(runs(&[2, 2, 2]), vec![(0, 3, 2)]);
        assert_eq!(
            runs(&[0, 1, 1, 0, 3, 4, 4, 4]),
            vec![(1, 2, 1), (4, 1, 3), (5, 3, 4)]
        );
    }

    #[test]
    fn a_worst_case_spectrogram_stays_within_the_shape_budget() {
        // Alternating levels never merge, so every cell is its own run.
        let row: Vec<usize> = (0..SPECTROGRAM_COLUMNS).map(|i| 1 + i % 2).collect();
        let total = runs(&row).len() * BINS;
        assert!(total <= 1600, "{total}");
    }
}
