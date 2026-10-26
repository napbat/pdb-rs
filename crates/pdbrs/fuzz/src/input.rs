//! Split a fuzz input into fields.
//!
//! A read past the end gives zero bytes, so every input selects a value.

/// A cursor over fuzz input bytes.
pub struct Input<'a> {
    data: &'a [u8],
}

impl<'a> Input<'a> {
    /// Start at the first byte of `data`.
    #[must_use]
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// Return `true` if no bytes remain.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Return the bytes that remain, and consume them.
    pub fn rest(&mut self) -> &'a [u8] {
        core::mem::take(&mut self.data)
    }

    /// Return the next `count` bytes, or fewer at the end of the input.
    pub fn bytes(&mut self, count: usize) -> &'a [u8] {
        let (head, tail) = self.data.split_at(count.min(self.data.len()));
        self.data = tail;
        head
    }

    /// Return the next byte, or 0 at the end of the input.
    pub fn u8(&mut self) -> u8 {
        self.bytes(1).first().copied().unwrap_or(0)
    }

    /// Return the next little-endian `u16`, with zero bytes past the end of the input.
    pub fn u16(&mut self) -> u16 {
        let mut word = [0; 2];
        let bytes = self.bytes(2);
        word[..bytes.len()].copy_from_slice(bytes);
        u16::from_le_bytes(word)
    }

    /// Return the next little-endian `u32`, with zero bytes past the end of the input.
    pub fn u32(&mut self) -> u32 {
        let mut word = [0; 4];
        let bytes = self.bytes(4);
        word[..bytes.len()].copy_from_slice(bytes);
        u32::from_le_bytes(word)
    }

    /// Return a part that has a 32-bit length prefix, or the rest of the input if it is shorter.
    pub fn part(&mut self) -> &'a [u8] {
        let length = usize::try_from(self.u32()).unwrap_or(usize::MAX);
        self.bytes(length)
    }
}
