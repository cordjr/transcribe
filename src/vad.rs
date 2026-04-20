const SAMPLE_RATE: usize = 16_000;
const WINDOW_SIZE: usize = SAMPLE_RATE / 10; // 100ms = 1600 samples
const DEFAULT_SILENCE_THRESHOLD: f32 = 0.01; // ~-40dB in linear scale
const DEFAULT_MIN_SILENCE_SECS: f32 = 2.0;
const DEFAULT_MAX_CHUNK_SECS: f32 = 60.0;

pub struct VadChunker {
    buffer: Vec<f32>,
    silence_threshold: f32,
    min_silence_samples: usize,
    max_chunk_samples: usize,
    silence_counter: usize,
    speech_detected: bool,
}

impl VadChunker {
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            silence_threshold: DEFAULT_SILENCE_THRESHOLD,
            min_silence_samples: (DEFAULT_MIN_SILENCE_SECS * SAMPLE_RATE as f32) as usize,
            max_chunk_samples: (DEFAULT_MAX_CHUNK_SECS * SAMPLE_RATE as f32) as usize,
            silence_counter: 0,
            speech_detected: false,
        }
    }

    /// Feed new samples into the chunker. Returns a speech chunk if a boundary is detected.
    pub fn feed(&mut self, samples: &[f32]) -> Option<Vec<f32>> {
        self.buffer.extend_from_slice(samples);

        // Check energy in the new samples using sliding windows
        let mut offset = if self.buffer.len() > samples.len() + WINDOW_SIZE {
            self.buffer.len() - samples.len() - WINDOW_SIZE
        } else {
            0
        };

        while offset + WINDOW_SIZE <= self.buffer.len() {
            let window = &self.buffer[offset..offset + WINDOW_SIZE];
            let rms = compute_rms(window);

            if rms > self.silence_threshold {
                self.speech_detected = true;
                self.silence_counter = 0;
            } else {
                self.silence_counter += WINDOW_SIZE;
            }

            offset += WINDOW_SIZE;
        }

        // Emit chunk on silence boundary after speech
        if self.speech_detected && self.silence_counter >= self.min_silence_samples {
            let split_point = self.buffer.len().saturating_sub(self.silence_counter);
            if split_point > 0 {
                let chunk: Vec<f32> = self.buffer.drain(..split_point).collect();
                self.buffer.clear(); // discard trailing silence
                self.silence_counter = 0;
                self.speech_detected = false;
                return Some(chunk);
            }
        }

        // Force emit on max duration
        if self.buffer.len() >= self.max_chunk_samples {
            let chunk: Vec<f32> = self.buffer.drain(..self.max_chunk_samples).collect();
            self.silence_counter = 0;
            self.speech_detected = false;
            return Some(chunk);
        }

        None
    }

    /// Flush remaining buffer (for graceful shutdown).
    pub fn flush(&mut self) -> Option<Vec<f32>> {
        if self.buffer.is_empty() {
            return None;
        }
        let chunk = std::mem::take(&mut self.buffer);
        self.silence_counter = 0;
        self.speech_detected = false;
        Some(chunk)
    }
}

fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}
