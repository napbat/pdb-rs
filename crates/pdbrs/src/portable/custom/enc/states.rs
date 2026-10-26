//! The Edit and Continue state machine state map.
//!
//! The blob holds:
//!
//! 1. The state count as a compressed unsigned integer.
//! 2. If the count is not 0: the negated syntax offset baseline as a
//!    compressed unsigned integer, and then for each state the state number
//!    as a compressed signed integer and the syntax offset minus the baseline
//!    as a compressed unsigned integer.
//!
//! The states are in syntax offset order. The Roslyn reader keeps a 1-byte
//! ordinal for the states with the same syntax offset, so at most 256 states
//! can have the same syntax offset.
//!
//! The Roslyn reader ignores bytes after the states. `Pdb2Pdb` copies the
//! alignment bytes of a native `MD2` record of kind 9 into the Portable PDB
//! (#14). This decoder accepts at most three zero bytes after the states for
//! this reason.

use alloc::vec::Vec;

use super::super::Entries;
use super::super::blob::{Reader, Writer, invalid, too_large};
use super::{read_baseline, read_syntax_offset, write_baseline, write_syntax_offset};
use crate::Result;

/// The largest number of zero bytes after the states.
const MAX_PADDING: usize = 3;

/// The largest number of states with the same syntax offset.
const MAX_SAME_OFFSET: usize = 256;

/// One state of a state machine.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StateMachineState {
    /// The state number. Suspension points have numbers from 0 up, and
    /// finally states have negative numbers.
    pub state_number: i32,
    /// The syntax offset of the `await` or `yield` of the state.
    pub syntax_offset: i32,
}

/// An Edit and Continue state machine state map.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EncStateMachineStateMap<'a> {
    /// The blob.
    bytes: &'a [u8],
    /// The states. [`EncStateMachineStateMap::parse`] checked them.
    states: &'a [u8],
    /// The number of states.
    len: usize,
    /// The syntax offset baseline.
    baseline: i32,
    /// The zero bytes after the states.
    padding: &'a [u8],
}

impl<'a> EncStateMachineStateMap<'a> {
    /// Decode a state machine state map blob.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof) if the
    ///   blob ends before the declared number of states.
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   compressed integer is not valid, if the syntax offsets decrease, if
    ///   more than 256 states have the same syntax offset, or if other bytes
    ///   than at most three zero bytes follow the states.
    pub fn parse(blob: &'a [u8]) -> Result<Self> {
        let mut reader = Reader::new(blob);
        let declared = reader.unsigned()?;
        let baseline = if declared == 0 {
            0
        } else {
            read_baseline(&mut reader)?
        };
        let start = reader.rest();
        let mut order = Order::default();
        let mut len = 0;
        for _ in 0..declared {
            order.check(read_state(&mut reader, baseline)?.syntax_offset)?;
            len += 1;
        }
        let padding = reader.rest();
        if padding.len() > MAX_PADDING || padding.iter().any(|&byte| byte != 0) {
            return Err(invalid("bytes follow the state machine states"));
        }
        Ok(Self {
            bytes: blob,
            states: &start[..start.len() - padding.len()],
            len,
            baseline,
            padding,
        })
    }

    /// Return the blob bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Return the syntax offset baseline, or 0 if there are no states.
    #[must_use]
    pub const fn syntax_offset_baseline(&self) -> i32 {
        self.baseline
    }

    /// Return the zero bytes after the states (at most three).
    #[must_use]
    pub const fn padding(&self) -> &'a [u8] {
        self.padding
    }

    /// Return the number of states.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Return `true` if there are no states.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return an iterator over the states.
    #[must_use]
    pub const fn iter(&self) -> Entries<'a, StateMachineState> {
        Entries::new(self.states, self.len, self.baseline, read_state)
    }

    /// Encode a state machine state map blob as Roslyn does, without padding.
    ///
    /// The baseline is the smallest of 0 and all syntax offsets.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if the
    ///   syntax offsets decrease, or if more than 256 states have the same
    ///   syntax offset.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if a
    ///   value does not fit in a compressed integer, or if an allocation fails.
    pub fn encode(
        states: impl IntoIterator<Item = StateMachineState, IntoIter: Clone>,
    ) -> Result<Vec<u8>> {
        let states = states.into_iter();
        let count = u32::try_from(states.clone().count())
            .map_err(|_| too_large("state machine state map has too many states"))?;
        let mut writer = Writer::new();
        writer.unsigned(count)?;
        if count == 0 {
            return Ok(writer.finish());
        }
        let baseline = states
            .clone()
            .map(|state| state.syntax_offset)
            .fold(0, i32::min);
        write_baseline(&mut writer, baseline)?;
        let mut order = Order::default();
        for state in states {
            order.check(state.syntax_offset)?;
            writer.signed(state.state_number)?;
            write_syntax_offset(&mut writer, state.syntax_offset, baseline)?;
        }
        Ok(writer.finish())
    }
}

iterable!(EncStateMachineStateMap => Entries<'a, StateMachineState>);

/// The order check of the syntax offsets.
#[derive(Default)]
struct Order {
    /// The last syntax offset.
    last: Option<i32>,
    /// The number of states with the last syntax offset.
    same: usize,
}

impl Order {
    /// Check the next syntax offset.
    fn check(&mut self, syntax_offset: i32) -> Result<()> {
        match self.last {
            Some(last) if syntax_offset < last => {
                return Err(invalid(
                    "state machine states are not in syntax offset order",
                ));
            }
            Some(last) if syntax_offset == last => self.same += 1,
            _ => self.same = 1,
        }
        if self.same > MAX_SAME_OFFSET {
            return Err(invalid(
                "more than 256 state machine states have the same syntax offset",
            ));
        }
        self.last = Some(syntax_offset);
        Ok(())
    }
}

/// Read one state.
fn read_state(reader: &mut Reader<'_>, baseline: i32) -> Result<StateMachineState> {
    let state_number = reader.signed()?;
    let syntax_offset = read_syntax_offset(reader, baseline)?;
    Ok(StateMachineState {
        state_number,
        syntax_offset,
    })
}
