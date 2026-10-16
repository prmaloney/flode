# Effects: plan and requirements

Seven effects are wired into the graph as nodes: distortion, compressor, reverb, delay, chorus,
phaser, and flanger. Each one's DSP is in `src/dsp/effects/`. This document is the plan they were
built from. It says how the pieces fit and what each effect has to do.

## How effects run

- An effect is a node with one audio input and one audio output. Its output can feed any node that
  takes audio: another effect, a filter, a mix, or an output slot. Only the oscillator takes no
  audio.
- Effects run once per sample on a slot's summed voices, after the per-voice stages. This is the
  bus. A stage runs on the bus if it is an effect or if anything on the bus feeds it, which the
  compiler works out in `place` in `src/patch/wiring.rs` and records in `StageSpec::bus`. A
  per-voice stage that a bus stage reads is tapped, so its voices are summed to feed the bus.
- A filter on the bus is one shared filter. A mix on the bus combines two summed signals, so it
  can sum two parallel effect chains. FM needs each note's pitch, so an FM mix on the bus passes
  its first input through.
- The voices are scaled by `VOICE_MIX` (0.25) first, so an effect sees the level the slot would
  have without it. The slot gain applies after the effects.
- The engine keeps four instances of each effect kind, and four of the bus filter, built in
  `Effects::new(sample_rate)`. A patch hands each bus stage an instance by its `pool` number. The
  glue that reads the knobs and calls the DSP is in `src/engine/effects.rs`.
- Tails keep ringing after the last voice ends, because the effects run on silence while the slot
  has stages.

## The contract for each DSP file

Each effect has one file in `src/dsp/effects/` with this shape:

- `<Name>Params`: plain-valued controls. The field list and ranges in the stub are fixed. Do not
  add, remove, or rename fields, because `src/engine/effects.rs` builds them.
- `<Name>::new(sample_rate: f32)`: builds the effect. It may allocate here and only here.
- `<Name>::reset()`: clears the tail and all filter memory.
- `<Name>::process(&mut self, input: Frame, params: &<Name>Params) -> Frame`: one stereo sample.

Rules for `process`:

- No allocation, no locks, no panics, no `Vec` growth. The engine test `no_alloc` runs all seven
  effects under a guard that aborts on any allocation.
- Output stays finite for any input in `[-8, 8]` and any params inside the documented ranges,
  including the extremes of every range and params that jump between calls.
- Smooth any param whose jump would click or zipper: levels, mix, delay times, filter
  frequencies. A change in delay time must not bend the pitch the way a naive jump does. Use the
  fractional `DelayLine` in `src/dsp/delay.rs` where it fits, and extend it only if you must.
- Flush denormals. A tail decaying to zero in a feedback loop must not stall the CPU. Add a tiny
  offset, or clamp values under about 1e-20 to zero.
- Handle any sample rate from 22050 to 192000 Hz. Scale delay lengths and filter coefficients by
  the rate.
- Dry/wet `mix` of 0.0 must return the dry input with no added delay (latency) and no coloring. A
  mix of 1.0 returns the wet signal only.
- Cost matters. The effect runs 44100 times a second per instance, with four instances of each in
  the pool. Avoid `powf`, `exp`, and `tan` per sample when a per-block or smoothed value will do.

Tests go in the same file, in `#[cfg(test)] mod tests`. They should measure behavior, not just
that something runs. Examples: frequency content with a Goertzel filter or a short DFT, decay time
by measuring the envelope, stability with extreme params, and that mix 0.0 is bit-exact dry.

## Style

- Match the surrounding code: doc comments on every public item, plain declarative sentences.
- No em dashes or en dashes anywhere, including comments and tests.
- Keep `#![allow(dead_code)]` out of the finished file. It is there only for the stub.
- Run `cargo fmt` before finishing.

## Distortion

A waveshaper with five curves, a drive, a tone control, and a dry/wet mix.

Params: `kind`, `drive` (0 to 1), `tone` (0 to 1), `level_db` (-60 to 0), `mix` (0 to 1).

- **Soft:** `tanh`-like saturation. Drive maps to pre-gain of 0 dB at 0.0 up to about 40 dB at 1.0.
- **Hard:** a clip at full scale after the same pre-gain.
- **Tube:** an asymmetric curve that makes even harmonics. It must not leave a DC offset, so add a
  DC blocker after it.
- **Fold:** a wavefolder. Peaks past full scale fold back, with several folds at high drive.
- **Crush:** bit depth reduction from 16 bits down to about 3, and a sample-rate hold from the full
  rate down to about a tenth of it. Drive sets both together. It aliases by design.
- **Tone:** a one-pole tilt after the curve. 0.5 is flat, 0.0 rolls off the highs, 1.0 lifts them
  and rolls off the lows a little. Keep it gentle.
- **Level:** scales the wet signal after the tone.
- **Aliasing:** run Soft, Hard, Tube, and Fold at 2x oversampling with a decent half-band filter or
  equivalent, so high drive does not turn into harsh aliasing. Crush runs at the plain rate.
- **Loudness:** at mid drive the wet signal should be within a few dB of the dry one for a
  full-scale sine, so the mix knob does not jump in level. Document the compensation you use.
- Stereo: the two sides run independently with identical settings. No state is shared between them.

## Compressor

A stereo-linked feed-forward compressor, with a multiband mode that has the OTT sound.

Params: `threshold_db`, `ratio`, `attack_ms`, `release_ms`, `makeup_db`, `mix`, `multiband`,
`depth`.

Single band mode (`multiband` false):

- A peak or RMS detector on the louder of the two sides, so the stereo image does not shift.
- A soft knee of about 6 dB around the threshold.
- Attack and release smoothing on the gain, in the log (dB) domain.
- `makeup_db` is added after compression. `mix` blends the compressed signal with the dry one, which
  is parallel compression. The dry signal must be time-aligned with the wet one. If you add
  lookahead, delay the dry by the same amount, and report it. Prefer no lookahead.
- At `ratio` 1.0 or a threshold above the signal, the output equals the input times the makeup gain.

Multiband mode (`multiband` true), the OTT sound:

- Split the signal into three bands with Linkwitz-Riley crossovers near 120 Hz and 2.5 kHz. The
  bands must sum back to the input with a flat magnitude response when no compression happens.
- Each band gets both downward compression above a high threshold and upward compression below a
  low one, which pulls quiet detail up. At full depth the result is dense and loud with the bright
  top end OTT is known for.
- `depth` (0 to 1) scales how much upward and downward compression each band does. At 0.0 the
  multiband path passes the signal through.
- `attack_ms` and `release_ms` still set the timing, scaled per band: faster for the highs, slower
  for the lows.
- `threshold_db` shifts both of each band's thresholds up or down together, and `ratio` sets the
  downward ratio. `makeup_db` and `mix` work as in single band mode.
- Make the three band gains independent of each other. Keep the stereo link inside each band.
- Switching `multiband` while sound plays must not click. Crossfade over a few milliseconds.
- Loudness: the gain stays bounded. Add a safety limit so a very quiet input with high depth cannot
  boost by more than about 30 dB.

## Reverb

Keep it basic: a stereo algorithmic reverb.

Params: `decay_s` (0.1 to 20), `size` (0 to 1), `damp` (0 to 1), `mix` (0 to 1).

- A Freeverb or feedback delay network design is fine. A FDN with 8 delay lines, a Hadamard or
  Householder mixing matrix, and a few input all-passes for diffusion gives a smoother tail.
- `decay_s` is the RT60. Measure it in a test: feed an impulse, and check the energy falls by 60
  dB in about `decay_s`, within 25 percent, for 0.5 s, 2 s, and 8 s.
- `size` scales the delay line lengths between about 0.5x and 2x. Changing it while sound plays
  must not click. Crossfade or modulate the read points smoothly. Slight modulation of the delay
  lengths is welcome to avoid metallic ringing.
- `damp` is a one-pole low pass inside the loop, so the high end decays faster than the low end.
- A little pre-diffusion keeps single impulses from sounding like a fluttering echo.
- Left and right come out decorrelated, so the tail is wide. A mono input still makes a stereo tail.
- The tail must stay stable at `decay_s` 20 with `damp` 0, and must decay to true silence (flush
  denormals) after the input stops.
- Allocate the delay memory in `new` for the sample rate, sized for the largest `size`.

## Delay

A stereo delay with feedback, damping, and a ping pong mode.

Params: `time_ms` (1 to 2000), `feedback` (0 to 0.95), `damp` (0 to 1), `mix` (0 to 1),
`ping_pong`.

- The caller resolves tempo sync. `time_ms` is always the time to use.
- Smooth `time_ms` with a slewed read point and an interpolated read, so turning the knob gives
  the classic tape pitch glide and never a click. The glide should settle in well under a second.
- `damp` is a one-pole low pass in the feedback path, so repeats darken.
- `feedback` up to 0.95 must stay stable, including with `damp` at 0.
- Normal mode: left and right delay independently, each fed by its own input side.
- Ping pong mode: the input enters one side, and each repeat crosses to the other side. The
  first repeat is on one side, the second on the other. Make the input go to the left delay, and
  feed back crosswise.
- Switching `ping_pong` while sound plays must not click.
- Allocate the lines in `new` for 2 seconds plus interpolation margin at the sample rate.
- A feedback path with a soft saturator is optional. Skip it if it adds cost without need.

## Chorus

Several short modulated delays mixed with the dry signal.

Params: `rate_hz` (0.05 to 200), `depth` (0 to 1), `spread` (0 to 1), `mix` (0 to 1).

- Three or four voices, each a modulated delay around 10 to 25 ms with its own LFO phase. The LFO
  is a sine or a triangle, with the phases spread evenly.
- `depth` scales the modulation swing, from nothing to about 4 ms at 1.0.
- `spread` sets how far apart the left and right sides sit. At 0.0 both sides get the same mix of
  voices. At 1.0 the voices pan across the field, and left and right see opposite modulation.
- Rates above about 10 Hz are an extreme vibrato and ring-mod-like effect. They must stay stable and
  must not read outside the delay buffer.
- No feedback. Interpolate the delay reads (linear is the minimum, cubic is better).
- Reading the dry signal in `mix` 0.0 must give an exact dry copy.

## Phaser

A chain of swept all-pass filters, summed with the dry signal.

Params: `rate_hz` (0.05 to 200), `depth` (0 to 1), `feedback` (0 to 0.95), `center_hz` (100 to
8000), `stages` (4, 6, 8, or 12), `spread` (0 to 1), `mix` (0 to 1).

- First-order all-pass stages in series. Each pair of stages makes one notch.
- An LFO sweeps the all-pass frequency exponentially (in octaves) around `center_hz`. `depth` sets
  the sweep width, up to about 3 octaves each way at 1.0. Keep the frequency under about 0.45 of
  the sample rate.
- Feedback takes the output of the chain back to its input, with a unit delay, scaled by
  `feedback`. It sharpens the notches. It must stay stable at 0.95 for every stage count.
- `spread` offsets the left and right LFO phases, from 0.0 (same) to 1.0 (opposite).
- Changing `stages` while sound plays must not click. Fade the extra stages in and out.
- A mix of 0.0 is the exact dry signal, and the wet is the chain output plus the dry, as in a
  classic phaser, so full wet still has the notches. Document the exact wet/dry gain law you use.
- The all-pass coefficient takes a `tan`. Compute it once per sample per side at most, or at a
  lower control rate with interpolation.

## Flanger

A very short modulated delay with feedback.

Params: `rate_hz` (0.05 to 200), `depth` (0 to 1), `feedback` (0 to 0.95), `delay_ms` (0.1 to
10), `spread` (0 to 1), `mix` (0 to 1).

- A modulated delay line with an interpolated read. The delay swings around `delay_ms` by
  `depth` times a fraction of it, and never goes below about 0.05 ms or past the buffer.
- Feedback takes the delayed signal back into the line, scaled by `feedback`. Allow the classic
  strong resonance up to 0.95 without blowing up. Add a gentle high-frequency roll-off in the loop
  so the tone stays musical.
- The LFO is a triangle or a sine. `spread` offsets the left and right LFO phases from 0.0 (same)
  to 1.0 (opposite).
- At a mix of 0.5 the dry and delayed signals have equal level, so the comb filter notches reach
  deep. At 0.0 the output is exact dry.
- Zero latency in the dry path.
- Interpolation quality matters here. Use cubic or an all-pass interpolator, because the delay is
  only a few samples long and linear interpolation darkens it audibly.
- Different from the chorus: shorter delay, feedback, and a comb sound. Do not share code that
  couples the two, since each file is built on its own.
