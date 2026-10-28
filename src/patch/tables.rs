//! Helpers for compiling custom wavetables. Cutting an audio file into frames can take a while on
//! a long file, and the patch compiles on every graph change, even a knob drag. So the cut frames
//! are remembered here, and a compile that sees the same file and slice mode gets them back.

use crate::dsp::frames::{slice_frames, SliceMode};
use crate::dsp::sample::SampleData;
use std::sync::{Arc, Mutex, Weak};

/// The most cut files the cache keeps.
const CACHE_LIMIT: usize = 4;

/// One remembered cut: the file it came from, the mode, and the frames.
struct Entry {
    /// A weak handle, so the cache never keeps a file alive. A dropped file's entry is stale and
    /// gets skipped, and its address can't be reused while the weak handle exists.
    sample: Weak<SampleData>,
    mode: SliceMode,
    frames: Arc<Vec<Vec<f32>>>,
}

/// The process-wide cache of cuts, oldest first.
fn cache() -> &'static Mutex<Vec<Entry>> {
    static CACHE: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
    &CACHE
}

/// The frames of `sample` cut as `mode` says, reusing a recent cut of the same loaded file. Call
/// it from the editor thread only.
pub fn cached_slice_frames(sample: &Arc<SampleData>, mode: SliceMode) -> Arc<Vec<Vec<f32>>> {
    let weak = Arc::downgrade(sample);
    if let Ok(cache) = cache().lock() {
        if let Some(hit) = cache
            .iter()
            .find(|e| Weak::ptr_eq(&e.sample, &weak) && e.mode == mode)
        {
            return hit.frames.clone();
        }
    }
    // Cut outside the lock so a slow cut doesn't hold it.
    let frames = Arc::new(slice_frames(sample, mode));
    if let Ok(mut cache) = cache().lock() {
        cache.retain(|e| e.sample.strong_count() > 0);
        while cache.len() >= CACHE_LIMIT {
            cache.remove(0);
        }
        cache.push(Entry {
            sample: weak,
            mode,
            frames: frames.clone(),
        });
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Arc<SampleData> {
        Arc::new(SampleData {
            sample_rate: 44100.0,
            channels: 1,
            frames: (0..8192).map(|i| (i as f32 * 0.05).sin()).collect(),
        })
    }

    /// Cutting the same loaded file twice gives the same frames without a second cut.
    #[test]
    fn the_same_file_and_mode_hit_the_cache() {
        let s = sample();
        let a = cached_slice_frames(&s, SliceMode::Fixed(2048));
        let b = cached_slice_frames(&s, SliceMode::Fixed(2048));
        assert!(Arc::ptr_eq(&a, &b));
        let c = cached_slice_frames(&s, SliceMode::Fixed(1024));
        assert!(!Arc::ptr_eq(&a, &c));
    }

    /// A different loaded file misses, even with the same contents.
    #[test]
    fn another_file_misses() {
        let a = cached_slice_frames(&sample(), SliceMode::Fixed(2048));
        let other = sample();
        let b = cached_slice_frames(&other, SliceMode::Fixed(2048));
        assert!(!Arc::ptr_eq(&a, &b));
    }
}
