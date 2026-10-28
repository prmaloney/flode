// The nih_export_vst3! macro (via vst3-com) expands with a trailing semicolon that
// newer rustc flags as a future-incompat lint. It's upstream in nih-plug, not our code.
#![allow(unknown_lints, semicolon_in_expressions_from_non_local_macros)]

//! Flode, a synth you build by wiring nodes together. This file is the plugin shell: it turns
//! host events and buffers into calls on the engine, and shares the patch and the meters with the
//! editor.

use nih_plug::prelude::*;
use nih_plug_iced::IcedState;
use rtrb::{Consumer, Producer, RingBuffer};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};

mod dsp;
mod editor;
mod engine;
mod groups;
mod nodes;
mod patch;
mod presets;

use dsp::wavetable::WaveTables;
use editor::graph::Graph;
use editor::piano::PianoEvent;
use engine::{Engine, Transport};
use patch::link::{AudioEnd, Meters, PatchLink};

/// Room for queued computer-keyboard notes. A full queue drops new notes.
const NOTE_QUEUE_LEN: usize = 256;

pub struct Flode {
    params: Arc<FlodeParams>,
    /// The synth. It is large, so it lives on the heap.
    engine: Box<Engine>,
    /// The editor's ends of the patch and meter channels.
    link: Arc<PatchLink>,
    /// The audio thread's ends of the same channels.
    audio: AudioEnd,
    /// Notes from the editor's computer keyboard. Drained on the audio thread in `process`.
    note_tx: Arc<Mutex<Producer<PianoEvent>>>,
    note_rx: Consumer<PianoEvent>,
    /// The note-ons the audio thread has received so far. The editor shows when it changes.
    notes_in: u32,
}

impl Default for Flode {
    fn default() -> Self {
        let (note_tx, note_rx) = RingBuffer::new(NOTE_QUEUE_LEN);
        let (link, audio) = PatchLink::new();
        Self {
            params: Arc::new(FlodeParams::default()),
            engine: Box::new(Engine::new(Arc::new(WaveTables::new()))),
            link,
            audio,
            note_tx: Arc::new(Mutex::new(note_tx)),
            note_rx,
            notes_in: 0,
        }
    }
}

/// Every knob lives on a node in the graph, so the project saves the graph instead of host
/// parameters. Two nodes of one kind keep their own values that way.
#[derive(Params)]
struct FlodeParams {
    /// Saves the editor window size with the project.
    #[persist = "editor-state"]
    editor_state: Arc<IcedState>,

    /// The node graph, so the wiring, the node positions, and every node's knobs survive save and
    /// reload.
    #[persist = "graph"]
    graph: Mutex<editor::saved::SavedGraph>,

    /// The four macro knobs, the only host parameters. The macro node shows them and drives
    /// whatever its pins are wired to, so the DAW can automate them.
    #[id = "macro1"]
    macro1: FloatParam,
    #[id = "macro2"]
    macro2: FloatParam,
    #[id = "macro3"]
    macro3: FloatParam,
    #[id = "macro4"]
    macro4: FloatParam,
}

/// A macro parameter: 0 to 100 percent, with a short glide so automation does not click.
fn macro_param(name: &'static str) -> FloatParam {
    FloatParam::new(name, 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(10.0))
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

impl Default for FlodeParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            graph: Mutex::new(editor::saved::SavedGraph::default()),
            macro1: macro_param("Macro 1"),
            macro2: macro_param("Macro 2"),
            macro3: macro_param("Macro 3"),
            macro4: macro_param("Macro 4"),
        }
    }
}

impl FlodeParams {
    /// Macro `index`, from 0. The macro node's knobs show these.
    fn macro_param(&self, index: usize) -> &FloatParam {
        match index {
            0 => &self.macro1,
            1 => &self.macro2,
            2 => &self.macro3,
            _ => &self.macro4,
        }
    }

    /// The next smoothed value of every macro, from 0.0 to 1.0. Call it once per sample.
    fn next_macro_levels(&self) -> [f32; nodes::macros::MACRO_COUNT] {
        std::array::from_fn(|index| self.macro_param(index).smoothed.next())
    }
}

impl Plugin for Flode {
    const NAME: &'static str = "Flode";
    const VENDOR: &'static str = "Patrick Maloney";
    const URL: &'static str = "https://github.com/prmaloney/flode";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        // A click on the key strip plays a note in the plugin too, and so does the computer
        // keyboard in Keyboard mode: Tab, not which kind of process this is, decides that now.
        editor::create(
            self.params.clone(),
            self.params.editor_state.clone(),
            Some(self.note_tx.clone()),
            self.link.clone(),
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.engine.set_sample_rate(buffer_config.sample_rate);
        // Set the patch from the saved graph. The editor keeps it current while it is open.
        if let Ok(saved) = self.params.graph.lock() {
            self.link.publish(Graph::from_saved(&saved).patch());
        }
        true
    }

    fn reset(&mut self) {
        self.engine.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let transport = context.transport();
        self.engine.begin_block(
            self.audio.patches.read(),
            Transport {
                beats: transport.pos_beats(),
                tempo: transport.tempo,
                playing: transport.playing,
            },
        );

        while let Ok(note) = self.note_rx.pop() {
            match note {
                PianoEvent::NoteOn { note, velocity } => {
                    self.notes_in = self.notes_in.wrapping_add(1);
                    self.engine.note_on(note, velocity)
                }
                PianoEvent::NoteOff { note } => self.engine.note_off(note),
            }
        }

        let mut next_event = context.next_event();
        for (sample_id, channel_samples) in buffer.iter_samples().enumerate() {
            // Handle every MIDI event scheduled for this sample.
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }
                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => {
                        self.notes_in = self.notes_in.wrapping_add(1);
                        self.engine.note_on(note, velocity)
                    }
                    NoteEvent::NoteOff { note, .. } => self.engine.note_off(note),
                    NoteEvent::MidiPitchBend { value, .. } => self.engine.pitch_bend(value),
                    NoteEvent::MidiCC { cc: 1, value, .. } => self.engine.mod_wheel(value),
                    NoteEvent::MidiChannelPressure { pressure, .. } => {
                        self.engine.aftertouch(pressure)
                    }
                    _ => {}
                }
                next_event = context.next_event();
            }

            self.engine.set_macros(self.params.next_macro_levels());
            let out = self.engine.process_sample(sample_id);
            for (channel, sample) in channel_samples.into_iter().enumerate() {
                *sample = match channel {
                    0 => out.l,
                    1 => out.r,
                    _ => out.mid(),
                };
            }
        }

        self.audio.meters.write(Meters {
            levels: self.engine.levels(),
            active_notes: self.engine.active_notes(),
            output: self.engine.take_output_peak(),
            notes_in: self.notes_in,
            spectra: self.engine.spectra(),
            monitors: self.engine.monitors(),
        });

        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for Flode {
    const CLAP_ID: &'static str = "app.flode.flode";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A MIDI-driven synth");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for Flode {
    // Must be exactly 16 bytes and unique to this plugin.
    const VST3_CLASS_ID: [u8; 16] = *b"FlodeSineSynth02";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nih_export_clap!(Flode);
nih_export_vst3!(Flode);
// The Audio Unit for Logic, wrapped around the CLAP entry point above. A no-op off macOS.
clap_wrapper::export_auv2!();

#[cfg(test)]
mod tests {
    use super::*;

    /// The four macros are the plugin's only host parameters, with stable ids and names.
    #[test]
    fn the_macros_are_the_host_parameters() {
        let params = FlodeParams::default();
        let map = params.param_map();
        let ids: Vec<&str> = map.iter().map(|(id, _, _)| id.as_str()).collect();
        assert_eq!(ids, ["macro1", "macro2", "macro3", "macro4"]);
        assert_eq!(map.len(), nodes::macros::MACRO_COUNT);
        for index in 0..nodes::macros::MACRO_COUNT {
            let param = params.macro_param(index);
            assert_eq!(param.name(), format!("Macro {}", index + 1));
            assert_eq!(param.default_normalized_value(), 0.0);
        }
    }

    /// The audio thread reads each macro's own value, and a changed value is smoothed to.
    #[test]
    fn each_macro_reads_its_own_value() {
        let params = FlodeParams::default();
        params.macro_param(2).smoothed.reset(0.5);
        let levels = params.next_macro_levels();
        assert_eq!(levels, [0.0, 0.0, 0.5, 0.0]);

        params.macro_param(0).smoothed.set_target(48000.0, 1.0);
        let first = params.next_macro_levels()[0];
        assert!(first > 0.0 && first < 1.0, "{first}");
    }
}
