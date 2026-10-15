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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

mod dsp;
mod editor;
mod engine;
mod nodes;
mod patch;

use dsp::wavetable::WaveTables;
use editor::graph::Graph;
use editor::piano::PianoEvent;
use engine::{Engine, Transport};
use patch::link::{AudioEnd, Meters, PatchLink};

/// Set by the standalone binary before it starts. Only standalone routes the computer keyboard to
/// the synth.
pub static STANDALONE: AtomicBool = AtomicBool::new(false);

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
}

impl Default for FlodeParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            graph: Mutex::new(editor::saved::SavedGraph::default()),
        }
    }
}

impl Plugin for Flode {
    const NAME: &'static str = "Flode";
    const VENDOR: &'static str = "Patrick Maloney";
    const URL: &'static str = "";
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
        let note_tx = STANDALONE
            .load(Ordering::Relaxed)
            .then(|| self.note_tx.clone());
        editor::create(
            self.params.clone(),
            self.params.editor_state.clone(),
            note_tx,
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
                PianoEvent::NoteOn { note, velocity } => self.engine.note_on(note, velocity),
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
                    NoteEvent::NoteOn { note, velocity, .. } => self.engine.note_on(note, velocity),
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
