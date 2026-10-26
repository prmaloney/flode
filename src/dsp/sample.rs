//! Loaded audio sample data for the Sampler node. Decoding happens off the audio thread, in a
//! background thread the editor starts. Once built, a `SampleData` never changes, so handing it
//! to the compiled patch is always an `Arc` clone, never a copy of the samples themselves.

use std::path::Path;

/// The file extensions a Sampler can load, lower case.
pub const EXTENSIONS: [&str; 10] = [
    "wav", "wave", "aif", "aiff", "flac", "mp3", "ogg", "m4a", "mp4", "aac",
];

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

fn is_wav(path: &Path) -> bool {
    matches!(extension_of(path).as_str(), "wav" | "wave")
}

/// Whether a file's extension is one a Sampler can load. A drop is checked against this so a
/// stray file never makes an empty Sampler.
pub fn is_audio_file(path: &Path) -> bool {
    EXTENSIONS.contains(&extension_of(path).as_str())
}

/// One loaded audio file, decoded to `f32`. Samples are interleaved: a stereo file's frame `n` is
/// `frames[2*n]` on the left and `frames[2*n+1]` on the right. A mono file has one value per
/// frame. Only WAV is supported today.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleData {
    pub sample_rate: f32,
    pub channels: u8,
    pub frames: Vec<f32>,
}

impl SampleData {
    /// How many frames the data holds, where a frame is one sample on every channel.
    pub fn len(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.frames.len() / self.channels as usize
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The left and right value at frame `n`. A mono file gives the same value on both sides. A
    /// frame past the end of the data is silence.
    pub fn frame(&self, n: usize) -> (f32, f32) {
        if n >= self.len() {
            return (0.0, 0.0);
        }
        match self.channels {
            1 => {
                let v = self.frames[n];
                (v, v)
            }
            _ => (self.frames[n * 2], self.frames[n * 2 + 1]),
        }
    }

    /// Loads an audio file, mono or stereo. A WAV file decodes 16, 24, or 32 bit integer PCM, or
    /// 32 bit float. Other formats (see `EXTENSIONS`) go through a general decoder. A file that
    /// cannot be read or decoded reports an error string instead of panicking.
    pub fn load(path: &Path) -> Result<Self, String> {
        if is_wav(path) {
            Self::load_wav(path)
        } else {
            Self::load_other(path)
        }
    }

    /// Decodes anything but a WAV file, as `f32`, with whatever channels the file has up to two.
    fn load_other(path: &Path) -> Result<Self, String> {
        use symphonia::core::audio::SampleBuffer;
        use symphonia::core::codecs::DecoderOptions;
        use symphonia::core::errors::Error;
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;

        let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
            let _ = hint.with_extension(extension);
        }
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                MediaSourceStream::new(Box::new(file), Default::default()),
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|error| error.to_string())?;
        let mut format = probed.format;
        let track = format
            .default_track()
            .ok_or_else(|| "the file has no audio".to_string())?;
        let track_id = track.id;
        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|error| error.to_string())?;

        let mut frames = Vec::new();
        let mut spec = None;
        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                // The end of the stream shows up as an unexpected end of file.
                Err(Error::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    break
                }
                Err(Error::ResetRequired) => break,
                Err(error) => return Err(error.to_string()),
            };
            if packet.track_id() != track_id {
                continue;
            }
            let decoded = match decoder.decode(&packet) {
                Ok(decoded) => decoded,
                // A damaged packet is skipped, like a player would.
                Err(Error::DecodeError(_)) => continue,
                Err(error) => return Err(error.to_string()),
            };
            let this = *decoded.spec();
            let channels = this.channels.count();
            if channels == 0 || channels > 2 {
                return Err(format!("{channels}-channel audio is not supported"));
            }
            spec = Some(this);
            let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, this);
            buffer.copy_interleaved_ref(decoded);
            frames.extend_from_slice(buffer.samples());
        }
        let spec = spec.ok_or_else(|| "the file has no audio".to_string())?;
        Ok(SampleData {
            sample_rate: spec.rate as f32,
            channels: spec.channels.count() as u8,
            frames,
        })
    }

    fn load_wav(path: &Path) -> Result<Self, String> {
        let mut reader = hound::WavReader::open(path).map_err(|error| format!("{}", error))?;
        let spec = reader.spec();
        if spec.channels == 0 || spec.channels > 2 {
            return Err(format!("{}-channel audio is not supported", spec.channels));
        }
        let frames: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .collect::<Result<Vec<f32>, _>>()
                .map_err(|error| format!("{}", error))?,
            hound::SampleFormat::Int => {
                // hound widens every integer depth to i32, keeping it at its own bit depth's
                // range, so the scale back to -1.0..1.0 uses that depth and not i32's.
                let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1).max(0)) as f32;
                reader
                    .samples::<i32>()
                    .map(|sample| sample.map(|sample| sample as f32 * scale))
                    .collect::<Result<Vec<f32>, _>>()
                    .map_err(|error| format!("{}", error))?
            }
        };
        Ok(SampleData {
            sample_rate: spec.sample_rate as f32,
            channels: spec.channels as u8,
            frames,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Writes a minimal WAV file at `path` with `bits` bit integer PCM, or 32 bit float when
    /// `float` is true, holding `samples` interleaved values already in the file's own units
    /// (so a caller writing 16 bit samples passes i16 range values as f32, etc).
    fn write_wav(path: &std::path::Path, channels: u16, bits: u16, float: bool, samples: &[f32]) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: 44100,
            bits_per_sample: bits,
            sample_format: if float {
                hound::SampleFormat::Float
            } else {
                hound::SampleFormat::Int
            },
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for &s in samples {
            if float {
                writer.write_sample(s).unwrap();
            } else {
                writer.write_sample(s as i32).unwrap();
            }
        }
        writer.finalize().unwrap();
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "flode-sample-test-{name}-{}.wav",
            std::process::id()
        ));
        path
    }

    #[test]
    fn decodes_16_bit_mono() {
        let path = temp_path("16-mono");
        write_wav(&path, 1, 16, false, &[0.0, 16384.0, -16384.0, 32767.0]);
        let data = SampleData::load(&path).unwrap();
        assert_eq!(data.channels, 1);
        assert_eq!(data.len(), 4);
        assert!((data.frame(0).0 - 0.0).abs() < 1e-4);
        assert!((data.frame(1).0 - 0.5).abs() < 1e-3);
        assert!((data.frame(2).0 + 0.5).abs() < 1e-3);
        assert!((data.frame(3).0 - 1.0).abs() < 1e-3);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn decodes_24_bit_stereo() {
        let path = temp_path("24-stereo");
        let full = (1i64 << 23) as f32 - 1.0;
        write_wav(&path, 2, 24, false, &[0.0, full, -full, 0.0]);
        let data = SampleData::load(&path).unwrap();
        assert_eq!(data.channels, 2);
        assert_eq!(data.len(), 2);
        assert!((data.frame(0).0 - 0.0).abs() < 1e-3);
        assert!((data.frame(0).1 - 1.0).abs() < 1e-3);
        assert!((data.frame(1).0 + 1.0).abs() < 1e-3);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn decodes_32_bit_int_mono() {
        let path = temp_path("32int-mono");
        let full = (1i64 << 31) as f32 - 1.0;
        write_wav(&path, 1, 32, false, &[full, -full]);
        let data = SampleData::load(&path).unwrap();
        assert!((data.frame(0).0 - 1.0).abs() < 1e-3);
        assert!((data.frame(1).0 + 1.0).abs() < 1e-3);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn decodes_32_bit_float_stereo() {
        let path = temp_path("32float-stereo");
        write_wav(&path, 2, 32, true, &[0.25, -0.5, 1.0, -1.0]);
        let data = SampleData::load(&path).unwrap();
        assert_eq!(data.channels, 2);
        assert_eq!(data.len(), 2);
        assert!((data.frame(0).0 - 0.25).abs() < 1e-6);
        assert!((data.frame(0).1 + 0.5).abs() < 1e-6);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_file_fails_cleanly() {
        let path = std::path::PathBuf::from("/no/such/flode/sample/file.wav");
        assert!(SampleData::load(&path).is_err());
    }

    #[test]
    fn a_non_wav_file_fails_cleanly() {
        let path = temp_path("not-a-wav");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"this is not a wav file, just some bytes")
            .unwrap();
        assert!(SampleData::load(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }

    /// An AIFF file, which a DAW exports as often as a WAV, goes through the general decoder.
    #[test]
    fn decodes_16_bit_aiff() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"FORM");
        bytes.extend_from_slice(&(4u32 + (8 + 18) + (8 + 16)).to_be_bytes());
        bytes.extend_from_slice(b"AIFF");
        bytes.extend_from_slice(b"COMM");
        bytes.extend_from_slice(&18u32.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(&4u32.to_be_bytes());
        bytes.extend_from_slice(&16u16.to_be_bytes());
        // 44100 Hz as an 80 bit extended float.
        bytes.extend_from_slice(&[0x40, 0x0e, 0xac, 0x44, 0, 0, 0, 0, 0, 0]);
        bytes.extend_from_slice(b"SSND");
        bytes.extend_from_slice(&16u32.to_be_bytes());
        bytes.extend_from_slice(&[0u8; 8]);
        for sample in [0i16, 16384, -16384, 32767] {
            bytes.extend_from_slice(&sample.to_be_bytes());
        }
        let mut path = temp_path("aiff");
        path.set_extension("aif");
        std::fs::write(&path, bytes).unwrap();
        let data = SampleData::load(&path).unwrap();
        assert_eq!(
            (data.channels, data.sample_rate, data.len()),
            (1, 44100.0, 4)
        );
        assert!((data.frame(1).0 - 0.5).abs() < 1e-3);
        assert!((data.frame(2).0 + 0.5).abs() < 1e-3);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn audio_files_are_told_by_their_extension() {
        for name in [
            "a.wav",
            "a.WAV",
            "clip.aif",
            "a.flac",
            "a.Mp3",
            "/x/y/a.m4a",
        ] {
            assert!(is_audio_file(Path::new(name)), "{name}");
        }
        for name in ["a.txt", "a", "a.als", "folder.wav.zip"] {
            assert!(!is_audio_file(Path::new(name)), "{name}");
        }
    }

    /// A file that has an audio extension but isn't audio fails cleanly in the general decoder.
    #[test]
    fn a_non_audio_mp3_fails_cleanly() {
        let mut path = temp_path("not-an-mp3");
        path.set_extension("mp3");
        std::fs::write(&path, b"this is not an mp3, just some bytes").unwrap();
        assert!(SampleData::load(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_mono_frame_reads_the_same_on_both_sides() {
        let data = SampleData {
            sample_rate: 44100.0,
            channels: 1,
            frames: vec![0.5, -0.5],
        };
        assert_eq!(data.frame(0), (0.5, 0.5));
        assert_eq!(data.frame(1), (-0.5, -0.5));
    }

    #[test]
    fn a_frame_past_the_end_is_silence() {
        let data = SampleData {
            sample_rate: 44100.0,
            channels: 2,
            frames: vec![1.0, 1.0],
        };
        assert_eq!(data.len(), 1);
        assert_eq!(data.frame(5), (0.0, 0.0));
    }
}
