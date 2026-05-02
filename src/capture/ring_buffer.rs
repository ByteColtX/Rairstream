use std::collections::VecDeque;

#[derive(Debug)]
pub struct FrameRingBuffer<T> {
    capacity: usize,
    queue: VecDeque<T>,
}

impl<T> FrameRingBuffer<T> {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            queue: VecDeque::with_capacity(capacity),
        }
    }

    pub fn push(&mut self, value: T) {
        if self.capacity == 0 {
            return;
        }

        if self.queue.len() == self.capacity {
            let _ = self.queue.pop_front();
        }
        self.queue.push_back(value);
    }

    pub fn pop(&mut self) -> Option<T> {
        self.queue.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::FrameRingBuffer;

    #[test]
    fn ring_buffer_discards_oldest_item_at_capacity() {
        let mut buffer = FrameRingBuffer::new(2);
        buffer.push(1);
        buffer.push(2);
        buffer.push(3);

        assert_eq!(buffer.pop(), Some(2));
        assert_eq!(buffer.pop(), Some(3));
    }

    #[test]
    fn ring_buffer_ignores_pushes_when_capacity_is_zero() {
        let mut buffer = FrameRingBuffer::new(0);
        buffer.push(1);

        assert_eq!(buffer.pop(), None);
    }
}
