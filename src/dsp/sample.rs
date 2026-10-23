//! Loaded audio sample data for the Sampler node. Decoding happens off the audio thread, in a
//! background thread the editor starts. Once built, a `SampleData` never changes, so handing it
//! to the compiled patch is always an `Arc` clone, never a copy of the samples themselves.

use std::path::Path;

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

    /// Loads a WAV file, decoding 16, 24, or 32 bit integer PCM, or 32 bit float, mono or stereo.
    /// Any other file, or one that cannot be read, reports an error string instead of panicking.
    pub fn load(path: &Path) -> Result<Self, String> {
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
