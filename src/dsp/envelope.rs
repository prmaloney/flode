//! The ADSR envelope generator. A voice runs one copy per envelope node, plus a bypass copy.

/// The short release a voice gets when no envelope shapes it. It is long enough to avoid a click.
pub const FADE_SECONDS: f32 = 0.01;

/// Where an envelope is in its cycle.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum EnvPhase {
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Per-sample envelope steps, recomputed once per block from the ADSR knobs.
#[derive(Clone, Copy)]
pub struct AdsrConfig {
    pub attack_step: f32,
    pub decay_step: f32,
    pub sustain: f32,
    pub release_seconds: f32,
}

impl AdsrConfig {
    /// No envelope in the chain: full level while held, with the short fade on release.
    pub const BYPASS: AdsrConfig = AdsrConfig {
        attack_step: 1.0,
        decay_step: 0.0,
        sustain: 1.0,
        release_seconds: FADE_SECONDS,
    };

    /// The steps for times in seconds and a sustain level from 0.0 to 1.0.
    pub fn from_values(
        attack: f32,
        decay: f32,
        sustain: f32,
        release: f32,
        sample_rate: f32,
    ) -> Self {
        let attack_samples = (attack * sample_rate).max(1.0);
        let decay_samples = (decay * sample_rate).max(1.0);
        Self {
            attack_step: 1.0 / attack_samples,
            decay_step: (1.0 - sustain) / decay_samples,
            sustain,
            release_seconds: release,
        }
    }
}

/// One envelope generator: an attack, decay, sustain, and release, run per voice.
#[derive(Clone, Copy)]
pub struct Adsr {
    pub phase: EnvPhase,
    /// The current level, from 0.0 to 1.0.
    pub level: f32,
    /// Level lost per sample while releasing.
    release_step: f32,
}

impl Adsr {
    pub const START: Adsr = Adsr {
        phase: EnvPhase::Attack,
        level: 0.0,
        release_step: 0.0,
    };

    /// Moves the envelope on by one sample.
    pub fn advance(&mut self, config: &AdsrConfig) {
        match self.phase {
            EnvPhase::Attack => {
                self.level += config.attack_step;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.phase = EnvPhase::Decay;
                }
            }
            EnvPhase::Decay => {
                self.level -= config.decay_step;
                if self.level <= config.sustain {
                    self.level = config.sustain;
                    self.phase = EnvPhase::Sustain;
                }
            }
            EnvPhase::Sustain => self.level = config.sustain,
            // A release stops at zero. Otherwise a copy that finished early keeps falling below zero
            // while the voice waits on a longer envelope, and a negative amplitude flips the sound.
            EnvPhase::Release => self.level = (self.level - self.release_step).max(0.0),
        }
    }

    /// Starts the release from the current level, falling to zero over `seconds`.
    pub fn release(&mut self, seconds: f32, sample_rate: f32) {
        if self.phase != EnvPhase::Release {
            self.phase = EnvPhase::Release;
            let samples = (seconds * sample_rate).max(1.0);
            self.release_step = self.level / samples;
        }
    }

    /// True once the release has run out.
    pub fn finished(&self) -> bool {
        self.phase == EnvPhase::Release && self.level <= 0.0
    }

    /// True once the release has started.
    pub fn releasing(&self) -> bool {
        self.phase == EnvPhase::Release
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A release never takes an envelope below zero, even while the voice waits on a longer one.
    #[test]
    fn a_finished_release_stays_at_zero() {
        let mut env = Adsr {
            phase: EnvPhase::Release,
            level: 0.1,
            release_step: 0.01,
        };
        for _ in 0..100 {
            env.advance(&AdsrConfig::BYPASS);
        }
        assert_eq!(env.level, 0.0);
    }

    /// The attack climbs to full, the decay falls to the sustain level, and it holds there.
    #[test]
    fn the_cycle_reaches_sustain_and_holds() {
        let config = AdsrConfig::from_values(0.001, 0.001, 0.5, 0.1, 1000.0);
        let mut env = Adsr::START;
        for _ in 0..10 {
            env.advance(&config);
        }
        assert_eq!(env.phase, EnvPhase::Sustain);
        assert_eq!(env.level, 0.5);
    }

    /// A release falls to zero over the time it was given.
    #[test]
    fn release_takes_the_given_time() {
        let config = AdsrConfig::from_values(0.001, 0.001, 1.0, 0.1, 1000.0);
        let mut env = Adsr::START;
        for _ in 0..10 {
            env.advance(&config);
        }
        env.release(0.1, 1000.0);
        for _ in 0..90 {
            env.advance(&config);
        }
        assert!(!env.finished());
        // Float error can leave a hair above zero on the 100th step, so allow a couple more.
        for _ in 0..12 {
            env.advance(&config);
        }
        assert!(env.finished());
    }
}
