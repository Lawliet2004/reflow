use std::collections::VecDeque;

/// Default ring buffer capacity: 60 seconds of 16kHz mono audio (960,000 samples).
pub const DEFAULT_AUDIO_RING_BUFFER_CAPACITY: usize = 60 * 16_000;

/// A bounded audio ring buffer that stores up to `capacity` samples
/// with drop-oldest semantics on overflow.
#[derive(Debug, Clone)]
pub struct AudioRingBuffer {
    buffer: VecDeque<f32>,
    capacity: usize,
    overflowed: bool,
    peak: f32,
    total_samples: u64,
}

impl Default for AudioRingBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_AUDIO_RING_BUFFER_CAPACITY)
    }
}

impl AudioRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity.max(1)),
            capacity: capacity.max(1),
            overflowed: false,
            peak: 0.0,
            total_samples: 0,
        }
    }

    pub fn extend_from_slice(&mut self, samples: &[f32]) {
        self.total_samples = self.total_samples.saturating_add(samples.len() as u64);
        self.peak = samples
            .iter()
            .filter(|sample| sample.is_finite())
            .map(|sample| sample.abs())
            .fold(self.peak, f32::max);
        if samples.is_empty() {
            return;
        }
        if samples.len() >= self.capacity {
            self.overflowed |= self.buffer.len() + samples.len() > self.capacity;
            self.buffer.clear();
            let start = samples.len() - self.capacity;
            self.buffer.extend(&samples[start..]);
            return;
        }

        let overflow = (self.buffer.len() + samples.len()).saturating_sub(self.capacity);
        if overflow > 0 {
            self.overflowed = true;
            self.buffer.drain(..overflow);
        }
        self.buffer.extend(samples);
    }

    pub fn take_all(&mut self) -> Vec<f32> {
        self.buffer.drain(..).collect()
    }

    pub fn to_vec(&self) -> Vec<f32> {
        self.buffer.iter().copied().collect()
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.overflowed = false;
        self.peak = 0.0;
        self.total_samples = 0;
    }

    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    pub fn peak(&self) -> f32 {
        self.peak
    }
    pub fn total_samples(&self) -> u64 {
        self.total_samples
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_accumulates_under_capacity() {
        let mut rb = AudioRingBuffer::new(10);
        rb.extend_from_slice(&[1.0, 2.0, 3.0]);
        assert_eq!(rb.len(), 3);
        assert_eq!(rb.to_vec(), vec![1.0, 2.0, 3.0]);

        rb.extend_from_slice(&[4.0, 5.0]);
        assert_eq!(rb.len(), 5);
        assert_eq!(rb.take_all(), vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        assert!(rb.is_empty());
    }

    #[test]
    fn ring_buffer_drops_oldest_on_overflow() {
        let mut rb = AudioRingBuffer::new(5);
        rb.extend_from_slice(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(rb.len(), 4);

        // Add 3 more samples: total 7 > capacity 5 -> drops [1.0, 2.0]
        rb.extend_from_slice(&[5.0, 6.0, 7.0]);
        assert_eq!(rb.len(), 5);
        assert_eq!(rb.take_all(), vec![3.0, 4.0, 5.0, 6.0, 7.0]);
    }

    #[test]
    fn ring_buffer_large_chunk_exceeding_capacity_keeps_latest() {
        let mut rb = AudioRingBuffer::new(4);
        rb.extend_from_slice(&[10.0, 20.0]);
        // Push 6 samples all at once
        rb.extend_from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(rb.len(), 4);
        assert_eq!(rb.take_all(), vec![3.0, 4.0, 5.0, 6.0]);
    }
}
