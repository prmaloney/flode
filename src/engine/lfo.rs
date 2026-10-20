//! The LFOs. Each one advances once per sample, and its level, from 0.0 to 1.0, drives any knob a
//! control wire joins it to.

use super::Transport;
use crate::dsp::rng::Rng;
use crate::dsp::wavetable::Waveform;
use crate::nodes::lfo::cycles_per_beat;
use crate::patch::control::{ControlLevels, MAX_LFOS};
use crate::patch::LfoSettings;

/// True while a Sync step past free locks an LFO to the host's playhead. That needs the host to
/// be playing.
fn follows_playhead(sync: usize, transport: &Transport) -> bool {
    sync >= 1 && transport.playing
}

/// The phase a one shot LFO's built-in wave reads for its held end value. It is just under one
/// cycle, because a wave at phase 1.0 is back at its start. A drawn curve reads its last point.
const ONE_SHOT_END: f32 = 0.9999;

/// Where a one shot LFO is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Shot {
    /// No note yet. It sits at the start of the cycle.
    Waiting,
    /// Playing its cycle.
    Running,
    /// The cycle ended. It holds the end value until the next note.
    Done,
}

/// The shape of `settings` at `phase`, from 0.0 to 1.0. A noise LFO has no shape, so the caller
/// does not ask.
fn shape_at(settings: &LfoSettings, phase: f32) -> f32 {
    match &settings.curve {
        Some(curve) => curve.eval(phase),
        // The wave runs from -1 to 1, and the level from 0 to 1.
        None => (settings.waveform.naive(phase) + 1.0) * 0.5,
    }
}

/// True if `settings` makes noise, which holds a random value in place of a shape.
fn is_noise(settings: &LfoSettings) -> bool {
    settings.curve.is_none() && settings.waveform == Waveform::Noise
}

pub struct Lfos {
    phase: [f32; MAX_LFOS],
    levels: [f32; MAX_LFOS],
    /// The random value a noise LFO holds, and the phase it last saw, to spot each new cycle.
    hold: [f32; MAX_LFOS],
    prev: [f32; MAX_LFOS],
    /// Where each LFO is in its one shot cycle. A free running LFO does not read it.
    shot: [Shot; MAX_LFOS],
}

impl Lfos {
    pub const fn new() -> Self {
        Self {
            phase: [0.0; MAX_LFOS],
            levels: [0.0; MAX_LFOS],
            hold: [0.5; MAX_LFOS],
            prev: [0.0; MAX_LFOS],
            shot: [Shot::Waiting; MAX_LFOS],
        }
    }

    /// Every LFO's level now.
    pub fn levels(&self) -> [f32; MAX_LFOS] {
        self.levels
    }

    /// Where every LFO is in its cycle, from 0.0 to 1.0.
    pub fn phases(&self) -> [f32; MAX_LFOS] {
        self.phase
    }

    /// Restarts every free running LFO that has Retrigger on, as a note starts. A noise LFO draws a
    /// new value too. An LFO that follows the playhead keeps its place in the song. A one shot LFO
    /// always starts its cycle again, whatever Retrigger and the playhead say.
    pub fn retrigger(
        &mut self,
        settings: &[Option<LfoSettings>; MAX_LFOS],
        levels: &ControlLevels,
        transport: &Transport,
        rng: &mut Rng,
    ) {
        for (lfo, settings) in settings.iter().enumerate() {
            let Some(settings) = settings else { continue };
            let sync = settings.sync.plain(levels).round() as usize;
            if settings.one_shot {
                self.shot[lfo] = Shot::Running;
            } else if !settings.retrigger || follows_playhead(sync, transport) {
                continue;
            }
            self.phase[lfo] = 0.0;
            self.prev[lfo] = 0.0;
            if is_noise(settings) {
                self.hold[lfo] = rng.unit();
            }
        }
    }

    /// Moves a one shot LFO on by one sample. It reports the phase it plays at, and a finished
    /// cycle reports 1.0.
    fn advance_one_shot(
        &mut self,
        lfo: usize,
        settings: &LfoSettings,
        levels: &ControlLevels,
        transport: &Transport,
        sample_rate: f32,
    ) {
        let level_at = |hold: f32, phase: f32| {
            if is_noise(settings) {
                hold
            } else {
                shape_at(settings, phase)
            }
        };
        match self.shot[lfo] {
            Shot::Waiting => {
                self.phase[lfo] = 0.0;
                self.prev[lfo] = 0.0;
                self.levels[lfo] = level_at(self.hold[lfo], 0.0);
            }
            Shot::Running => {
                let phase = self.phase[lfo];
                self.levels[lfo] = level_at(self.hold[lfo], phase);
                // With a note division and a host tempo the cycle is that division long. Otherwise
                // it takes 1 / rate seconds.
                let sync = settings.sync.plain(levels).round() as usize;
                let cycles_per_second = match (sync >= 1, transport.tempo) {
                    (true, Some(bpm)) => bpm as f32 / 60.0 * cycles_per_beat(sync),
                    _ => settings.rate.plain(levels),
                };
                let next = phase + cycles_per_second / sample_rate;
                if next >= 1.0 {
                    self.shot[lfo] = Shot::Done;
                    self.phase[lfo] = 1.0;
                } else {
                    self.phase[lfo] = next;
                }
            }
            Shot::Done => {
                self.phase[lfo] = 1.0;
                let end = if settings.curve.is_some() {
                    1.0
                } else {
                    ONE_SHOT_END
                };
                self.levels[lfo] = level_at(self.hold[lfo], end);
            }
        }
    }

    /// Moves every LFO on by one sample. `levels` is what its rate and sync knobs read.
    pub fn advance(
        &mut self,
        settings: &[Option<LfoSettings>; MAX_LFOS],
        levels: &ControlLevels,
        transport: &Transport,
        sample_id: usize,
        sample_rate: f32,
        rng: &mut Rng,
    ) {
        for (lfo, settings) in settings.iter().enumerate() {
            let Some(settings) = settings else {
                self.levels[lfo] = 0.0;
                continue;
            };
            if settings.one_shot {
                self.advance_one_shot(lfo, settings, levels, transport, sample_rate);
                continue;
            }
            // A Sync step past free locks the LFO to the playhead, so it follows the song while it
            // plays. Otherwise, or without a host tempo, it runs at its rate.
            let sync = settings.sync.plain(levels).round() as usize;
            let beats = match (
                follows_playhead(sync, transport),
                transport.beats,
                transport.tempo,
            ) {
                (true, Some(beat), Some(bpm)) => {
                    Some(beat + sample_id as f64 * bpm / (60.0 * sample_rate as f64))
                }
                _ => None,
            };
            let phase = match beats {
                Some(beats) => {
                    let cycles = beats * cycles_per_beat(sync) as f64;
                    let phase = cycles.rem_euclid(1.0) as f32;
                    self.phase[lfo] = phase;
                    phase
                }
                None => {
                    let phase = self.phase[lfo];
                    let rate = settings.rate.plain(levels);
                    self.phase[lfo] = (phase + rate / sample_rate).fract();
                    phase
                }
            };
            // Noise holds a random value for each cycle, so its rate is how often it changes.
            let wrapped = phase < self.prev[lfo];
            self.prev[lfo] = phase;
            self.levels[lfo] = if is_noise(settings) {
                if wrapped {
                    self.hold[lfo] = rng.unit();
                }
                self.hold[lfo]
            } else {
                shape_at(settings, phase)
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::curve::Curve;
    use crate::nodes::knobs::KnobId;
    use crate::patch::control::Param;

    const RATE: f32 = 44100.0;

    fn lfo(waveform: Waveform, rate: f32, sync: f32) -> [Option<LfoSettings>; MAX_LFOS] {
        let mut settings = [None; MAX_LFOS];
        settings[0] = Some(LfoSettings {
            waveform,
            rate: Param::fixed(KnobId::LfoRate, rate),
            sync: Param::fixed(KnobId::LfoSync, sync),
            retrigger: false,
            one_shot: false,
            curve: None,
        });
        settings
    }

    fn one_shot(mut settings: [Option<LfoSettings>; MAX_LFOS]) -> [Option<LfoSettings>; MAX_LFOS] {
        settings[0].as_mut().unwrap().one_shot = true;
        settings
    }

    fn drawn(
        mut settings: [Option<LfoSettings>; MAX_LFOS],
        points: &[[f32; 2]],
    ) -> [Option<LfoSettings>; MAX_LFOS] {
        settings[0].as_mut().unwrap().curve = Curve::from_points(points);
        settings
    }

    /// Tells every LFO a note started.
    fn note(lfos: &mut Lfos, settings: &[Option<LfoSettings>; MAX_LFOS], transport: &Transport) {
        lfos.retrigger(
            settings,
            &ControlLevels::default(),
            transport,
            &mut Rng::default(),
        );
    }

    /// A transport that is playing at 120 bpm, half a beat in.
    fn playing() -> Transport {
        Transport {
            beats: Some(0.5),
            tempo: Some(120.0),
            playing: true,
        }
    }

    /// `run` that keeps going from sample `from`, so a long run can be done in pieces.
    fn run_from(
        lfos: &mut Lfos,
        settings: &[Option<LfoSettings>; MAX_LFOS],
        transport: &Transport,
        from: usize,
        samples: usize,
    ) -> Vec<f32> {
        let mut rng = Rng::default();
        let levels = ControlLevels::default();
        (from..from + samples)
            .map(|n| {
                lfos.advance(settings, &levels, transport, n, RATE, &mut rng);
                lfos.levels()[0]
            })
            .collect()
    }

    /// A one shot saw at 100 Hz, which is a cycle of 441 samples.
    fn one_shot_saw() -> [Option<LfoSettings>; MAX_LFOS] {
        one_shot(lfo(Waveform::Saw, 100.0, 0.0))
    }

    /// Before a note, a one shot LFO sits at the start of its cycle.
    #[test]
    fn a_one_shot_waits_at_the_start_before_a_note() {
        let mut lfos = Lfos::new();
        let out = run(&mut lfos, &one_shot_saw(), &Transport::default(), 1000);
        assert!(out.iter().all(|&level| level == 0.0));
        assert_eq!(lfos.phases()[0], 0.0);
    }

    /// From a note it runs one cycle, then holds the shape's end value. A saw ends high.
    #[test]
    fn a_one_shot_runs_one_cycle_then_holds_the_end() {
        let settings = one_shot_saw();
        let transport = Transport::default();
        let mut lfos = Lfos::new();
        note(&mut lfos, &settings, &transport);
        let out = run(&mut lfos, &settings, &transport, 2000);
        assert_eq!(out[0], 0.0);
        assert!((out[220] - 0.5).abs() < 0.01, "{}", out[220]);
        assert!(out[440] > 0.99, "{}", out[440]);
        assert!(out.windows(2).take(440).all(|pair| pair[1] >= pair[0]));
        // Once the cycle ends, the level stays at its end for good.
        assert!(out[445..].iter().all(|&level| level == out[445]));
        assert!(out[445] > 0.99, "{}", out[445]);
        assert_eq!(lfos.phases()[0], 1.0);
    }

    /// A new note starts the cycle again, also while it holds.
    #[test]
    fn a_new_note_restarts_a_one_shot() {
        let settings = one_shot_saw();
        let transport = Transport::default();
        let mut lfos = Lfos::new();
        note(&mut lfos, &settings, &transport);
        run(&mut lfos, &settings, &transport, 2000);
        note(&mut lfos, &settings, &transport);
        let out = run(&mut lfos, &settings, &transport, 600);
        assert_eq!(out[0], 0.0);
        assert!((out[220] - 0.5).abs() < 0.01, "{}", out[220]);
        assert!(out[599] > 0.99);
        // A note in the middle of a cycle restarts it too.
        note(&mut lfos, &settings, &transport);
        run(&mut lfos, &settings, &transport, 200);
        note(&mut lfos, &settings, &transport);
        assert_eq!(run(&mut lfos, &settings, &transport, 1)[0], 0.0);
    }

    /// One shot restarts at a note with Retrigger off.
    #[test]
    fn a_one_shot_restarts_without_the_retrigger_switch() {
        let settings = one_shot_saw();
        assert!(!settings[0].unwrap().retrigger);
        let level = level_after_a_note(&settings, &Transport::default());
        assert!(level < 0.01, "{level}");
    }

    /// One shot does not follow the playhead, even with a note division and a playing host. A
    /// note starts it from the beginning.
    #[test]
    fn a_one_shot_ignores_the_playhead_lock() {
        let settings = one_shot(lfo(Waveform::Saw, 1.0, 3.0));
        let level = level_after_a_note(&settings, &playing());
        assert!(level < 0.01, "{level}");
    }

    /// With a note division and a host tempo, the cycle is that division long. A quarter note at
    /// 120 bpm is half a second, which is 22050 samples. The host does not have to play.
    #[test]
    fn a_synced_one_shot_takes_the_division_at_the_host_tempo() {
        let settings = one_shot(lfo(Waveform::Saw, 100.0, 3.0));
        let transport = Transport {
            beats: None,
            tempo: Some(120.0),
            playing: false,
        };
        let mut lfos = Lfos::new();
        note(&mut lfos, &settings, &transport);
        let out = run(&mut lfos, &settings, &transport, 22050);
        assert!((out[11025] - 0.5).abs() < 0.01, "{}", out[11025]);
        assert!(out[22049] > 0.99, "{}", out[22049]);
        // After the cycle it holds. A free running 100 Hz LFO would have wrapped 50 times.
        let held = run_from(&mut lfos, &settings, &transport, 22050, 5000);
        assert!(held[10..]
            .iter()
            .all(|&level| level == held[10] && level > 0.99));
    }

    /// With a note division but no host tempo, the cycle takes 1 / rate seconds.
    #[test]
    fn a_synced_one_shot_without_a_tempo_uses_the_rate() {
        let settings = one_shot(lfo(Waveform::Saw, 100.0, 3.0));
        let transport = Transport::default();
        let mut lfos = Lfos::new();
        note(&mut lfos, &settings, &transport);
        let out = run(&mut lfos, &settings, &transport, 600);
        assert!(out[445] > 0.99);
        assert!(out[445..].iter().all(|&level| level == out[445]));
    }

    /// Each built-in wave holds the value it has at the end of its cycle.
    #[test]
    fn a_one_shot_holds_the_end_of_each_wave() {
        for (wave, end) in [
            (Waveform::Sine, 0.5),
            (Waveform::Saw, 1.0),
            (Waveform::Square, 0.0),
            (Waveform::Triangle, 0.0),
        ] {
            let settings = one_shot(lfo(wave, 100.0, 0.0));
            let mut lfos = Lfos::new();
            note(&mut lfos, &settings, &Transport::default());
            let out = run(&mut lfos, &settings, &Transport::default(), 1000);
            assert!((out[999] - end).abs() < 0.01, "{wave:?} {}", out[999]);
        }
    }

    /// A noise one shot draws one value at each note and holds it.
    #[test]
    fn a_one_shot_noise_draws_one_value_per_note() {
        let settings = one_shot(lfo(Waveform::Noise, 100.0, 0.0));
        let transport = Transport::default();
        let mut lfos = Lfos::new();
        let mut rng = Rng::default();
        let mut seen = Vec::new();
        for _ in 0..4 {
            lfos.retrigger(&settings, &ControlLevels::default(), &transport, &mut rng);
            let out = run(&mut lfos, &settings, &transport, 1500);
            assert!(out.iter().all(|&level| level == out[0]));
            seen.push(out[0]);
        }
        assert!(seen.windows(2).any(|pair| pair[0] != pair[1]), "{seen:?}");
    }

    /// A drawn curve is the LFO's level, straight between its points. At 1 Hz, sample `n` is at
    /// phase n / 44100.
    #[test]
    fn a_custom_curve_is_the_level() {
        let settings = drawn(
            lfo(Waveform::Sine, 1.0, 0.0),
            &[[0.0, 0.0], [0.25, 1.0], [1.0, 0.5]],
        );
        let mut lfos = Lfos::new();
        let out = run(&mut lfos, &settings, &Transport::default(), 44100);
        assert!(out[0].abs() < 1e-4);
        assert!((out[5512] - 0.5).abs() < 1e-3, "{}", out[5512]);
        assert!((out[11025] - 1.0).abs() < 1e-3, "{}", out[11025]);
        assert!((out[27562] - 0.75).abs() < 1e-3, "{}", out[27562]);
        assert!((out[44099] - 0.5).abs() < 1e-2, "{}", out[44099]);
    }

    /// A one shot custom curve ends on the curve's last point.
    #[test]
    fn a_custom_one_shot_ends_on_the_last_point() {
        let settings = one_shot(drawn(
            lfo(Waveform::Sine, 100.0, 0.0),
            &[[0.0, 0.2], [0.5, 1.0], [1.0, 0.35]],
        ));
        let mut lfos = Lfos::new();
        note(&mut lfos, &settings, &Transport::default());
        let out = run(&mut lfos, &settings, &Transport::default(), 1000);
        assert!((out[0] - 0.2).abs() < 1e-4);
        assert!((out[999] - 0.35).abs() < 1e-5, "{}", out[999]);
    }

    /// With one shot off, the LFO runs as it did, and a note leaves a plain LFO alone.
    #[test]
    fn an_lfo_without_one_shot_keeps_running() {
        let settings = lfo(Waveform::Saw, 100.0, 0.0);
        let mut lfos = Lfos::new();
        let out = run(&mut lfos, &settings, &Transport::default(), 2000);
        let wraps = out.windows(2).filter(|pair| pair[1] < pair[0]).count();
        assert_eq!(wraps, 4);
    }

    fn retriggering(
        mut settings: [Option<LfoSettings>; MAX_LFOS],
    ) -> [Option<LfoSettings>; MAX_LFOS] {
        settings[0].as_mut().unwrap().retrigger = true;
        settings
    }

    /// A free running saw, run partway through its cycle, then told a note started.
    fn level_after_a_note(
        settings: &[Option<LfoSettings>; MAX_LFOS],
        transport: &Transport,
    ) -> f32 {
        let mut lfos = Lfos::new();
        run(&mut lfos, settings, transport, 12345);
        lfos.retrigger(
            settings,
            &ControlLevels::default(),
            transport,
            &mut Rng::default(),
        );
        run(&mut lfos, settings, transport, 1)[0]
    }

    /// With Retrigger on, a note puts a free running LFO back at the start of its cycle.
    #[test]
    fn a_note_restarts_a_retriggering_lfo() {
        let saw = retriggering(lfo(Waveform::Saw, 3.0, 0.0));
        let level = level_after_a_note(&saw, &Transport::default());
        assert!(level < 0.01, "{level}");
    }

    /// With it off, a note leaves the LFO where it was.
    #[test]
    fn a_note_leaves_a_plain_lfo_alone() {
        let saw = lfo(Waveform::Saw, 3.0, 0.0);
        let level = level_after_a_note(&saw, &Transport::default());
        // 12345 samples at 3 Hz is 0.84 of a cycle.
        assert!(level > 0.8, "{level}");
    }

    /// A synced LFO follows the playhead, so a note does not move it.
    #[test]
    fn a_note_does_not_move_a_synced_lfo() {
        let synced = retriggering(lfo(Waveform::Saw, 1.0, 3.0));
        let transport = Transport {
            beats: Some(0.5),
            tempo: Some(120.0),
            playing: true,
        };
        let level = level_after_a_note(&synced, &transport);
        assert!((level - 0.5).abs() < 0.01, "{level}");
    }

    /// A noise LFO draws a fresh value at a note.
    #[test]
    fn a_note_draws_a_new_noise_value() {
        let noise = retriggering(lfo(Waveform::Noise, 1.0, 0.0));
        let mut lfos = Lfos::new();
        let before = run(&mut lfos, &noise, &Transport::default(), 100)[99];
        let mut seen = false;
        let mut rng = Rng::default();
        for _ in 0..5 {
            lfos.retrigger(
                &noise,
                &ControlLevels::default(),
                &Transport::default(),
                &mut rng,
            );
            seen |= run(&mut lfos, &noise, &Transport::default(), 1)[0] != before;
        }
        assert!(seen);
    }

    fn run(
        lfos: &mut Lfos,
        settings: &[Option<LfoSettings>; MAX_LFOS],
        transport: &Transport,
        samples: usize,
    ) -> Vec<f32> {
        let mut rng = Rng::default();
        let levels = ControlLevels::default();
        (0..samples)
            .map(|n| {
                lfos.advance(settings, &levels, transport, n, RATE, &mut rng);
                lfos.levels()[0]
            })
            .collect()
    }

    #[test]
    fn a_missing_lfo_makes_no_signal() {
        let mut lfos = Lfos::new();
        let out = run(&mut lfos, &[None; MAX_LFOS], &Transport::default(), 100);
        assert!(out.iter().all(|&level| level == 0.0));
    }

    /// A free-running saw at 100 Hz wraps about 100 times a second.
    #[test]
    fn a_free_running_lfo_follows_its_rate() {
        let mut lfos = Lfos::new();
        let out = run(
            &mut lfos,
            &lfo(Waveform::Saw, 100.0, 0.0),
            &Transport::default(),
            44100,
        );
        let wraps = out.windows(2).filter(|pair| pair[1] < pair[0]).count();
        assert!((99..=101).contains(&wraps), "{wraps}");
        assert!(out.iter().all(|level| (0.0..=1.0).contains(level)));
    }

    /// A noise LFO holds one value for a whole cycle, then picks another.
    #[test]
    fn noise_holds_between_cycles() {
        let mut lfos = Lfos::new();
        // 200 Hz, the knob's top, is a cycle about every 220 samples at 44.1 kHz.
        let out = run(
            &mut lfos,
            &lfo(Waveform::Noise, 200.0, 0.0),
            &Transport::default(),
            450,
        );
        assert!(out[10..200].iter().all(|&level| level == out[10]));
        assert_ne!(out[10], out[300]);
    }

    /// With the host playing, a synced LFO takes its phase from the playhead.
    #[test]
    fn a_synced_lfo_follows_the_playhead() {
        let mut lfos = Lfos::new();
        // Step 3 is a quarter note: one cycle per beat. Half a beat in, a saw sits at its middle.
        let transport = Transport {
            beats: Some(0.5),
            tempo: Some(120.0),
            playing: true,
        };
        let out = run(&mut lfos, &lfo(Waveform::Saw, 1.0, 3.0), &transport, 1);
        assert!((out[0] - 0.5).abs() < 1e-4, "{}", out[0]);
    }

    /// A stopped host leaves a synced LFO free running.
    #[test]
    fn a_stopped_host_leaves_sync_free_running() {
        let mut lfos = Lfos::new();
        let transport = Transport {
            beats: Some(0.5),
            tempo: Some(120.0),
            playing: false,
        };
        let out = run(&mut lfos, &lfo(Waveform::Saw, 1.0, 3.0), &transport, 2);
        assert!(out[1] < 0.01, "{}", out[1]);
    }
}
