//! The LFOs. Each one advances once per sample, and its level, from 0.0 to 1.0, drives any knob a
//! control wire joins it to.

use super::Transport;
use crate::dsp::rng::Rng;
use crate::dsp::wavetable::Waveform;
use crate::nodes::lfo::cycles_per_beat;
use crate::patch::control::{ControlLevels, MAX_LFOS};
use crate::patch::LfoSettings;

pub struct Lfos {
    phase: [f32; MAX_LFOS],
    levels: [f32; MAX_LFOS],
    /// The random value a noise LFO holds, and the phase it last saw, to spot each new cycle.
    hold: [f32; MAX_LFOS],
    prev: [f32; MAX_LFOS],
}

impl Lfos {
    pub const fn new() -> Self {
        Self {
            phase: [0.0; MAX_LFOS],
            levels: [0.0; MAX_LFOS],
            hold: [0.5; MAX_LFOS],
            prev: [0.0; MAX_LFOS],
        }
    }

    /// Every LFO's level now.
    pub fn levels(&self) -> [f32; MAX_LFOS] {
        self.levels
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
            // A Sync step past free locks the LFO to the playhead, so it follows the song while it
            // plays. Otherwise, or without a host tempo, it runs at its rate.
            let sync = settings.sync.plain(levels).round() as usize;
            let beats = match (transport.playing, transport.beats, transport.tempo) {
                (true, Some(beat), Some(bpm)) if sync >= 1 => {
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
            self.levels[lfo] = if settings.waveform == Waveform::Noise {
                if wrapped {
                    self.hold[lfo] = rng.unit();
                }
                self.hold[lfo]
            } else {
                (settings.waveform.naive(phase) + 1.0) * 0.5
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::knobs::KnobId;
    use crate::patch::control::Param;

    const RATE: f32 = 44100.0;

    fn lfo(waveform: Waveform, rate: f32, sync: f32) -> [Option<LfoSettings>; MAX_LFOS] {
        let mut settings = [None; MAX_LFOS];
        settings[0] = Some(LfoSettings {
            waveform,
            rate: Param::fixed(KnobId::LfoRate, rate),
            sync: Param::fixed(KnobId::LfoSync, sync),
        });
        settings
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
