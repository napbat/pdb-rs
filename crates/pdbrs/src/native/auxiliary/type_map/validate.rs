//! Reference validation of a type map stream.
//!
//! [`TypeMapStream::validate`] checks the map values, the record offsets of
//! an object map and the function ID pairs. It reports each broken rule as a
//! [`TypeMapViolation`] and does not stop at the first one. A map value of 0
//! marks an input index that the linker did not map, and is valid.
//!
//! The kind of an input record of an object map is only in the object file.
//! Thus a value of an object map can be checked only against the index range
//! of the TPI and the IPI stream together. The function ID pairs tie the map
//! values of the input function IDs to the IPI stream.

use alloc::vec::Vec;

use super::{TypeMapKind, TypeMapStream};
use crate::Result;
use crate::native::codeview::id_record::{IdRecord, IdTable};
use crate::native::codeview::index::{FIRST_RECORD_INDEX, IdIndex, TypeIndex};
use crate::native::output::no_memory;

/// The size of the length field and the kind field of a record. A record
/// offset of an object map grows by at least this value.
const MIN_RECORD_SIZE: u32 = 4;

/// A rule that a type map stream can break.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum TypeMapRule {
    /// A value of the type map of a type server map is not 0 and not a TPI
    /// index.
    TypeValue,
    /// A value of the ID map of a type server map is not 0 and not an IPI
    /// index.
    IdValue,
    /// A value of an object map is not 0 and not an index of the TPI or the
    /// IPI stream.
    ObjectValue,
    /// The first input index of an object map is below `0x1000`, or the
    /// input index range does not fit in 32 bits.
    FirstIndex,
    /// The record offsets of an object map do not start at 0, or an offset
    /// does not grow by a multiple of 4 that is at least 4.
    RecordOffset,
    /// The input function ID of a function ID pair is not an input ID of the
    /// map.
    FunctionInput,
    /// The type of a function ID pair is not a TPI index.
    FunctionType,
    /// The map value of the input function ID of a pair is not an
    /// `LF_FUNC_ID` or `LF_MFUNC_ID` record with the type of the pair.
    FunctionMapping,
}

impl TypeMapRule {
    /// Return a short description of the rule.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::TypeValue => "type map value is not a TPI index",
            Self::IdValue => "type map ID value is not an IPI index",
            Self::ObjectValue => "object map value is not a TPI or IPI index",
            Self::FirstIndex => "object map input index range is not valid",
            Self::RecordOffset => "object map record offsets do not grow by whole records",
            Self::FunctionInput => "type map function ID is not an input ID",
            Self::FunctionType => "type map function type is not a TPI index",
            Self::FunctionMapping => "type map function ID does not map to its function type",
        }
    }
}

/// One broken rule in a type map stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct TypeMapViolation {
    /// The broken rule.
    pub rule: TypeMapRule,
    /// The position of the value in its map, of the record offset, or of the
    /// function ID pair. It is 0 for [`TypeMapRule::FirstIndex`].
    pub position: usize,
    /// The value that breaks the rule.
    pub value: u32,
}

/// The data from other streams that [`TypeMapStream::validate`] uses.
#[derive(Clone, Copy, Debug)]
pub struct TypeMapContext<'c> {
    /// The index after the last TPI record.
    type_index_end: TypeIndex,
    /// The index after the last IPI record.
    id_index_end: IdIndex,
    /// The IPI records.
    ids: Option<&'c IdTable<'c>>,
}

impl<'c> TypeMapContext<'c> {
    /// Create a context with the index ends of the TPI and IPI streams.
    #[must_use]
    pub const fn new(type_index_end: TypeIndex, id_index_end: IdIndex) -> Self {
        Self {
            type_index_end,
            id_index_end,
            ids: None,
        }
    }

    /// Add the IPI records for [`TypeMapRule::FunctionMapping`].
    #[must_use]
    pub const fn with_ids(mut self, ids: &'c IdTable<'c>) -> Self {
        self.ids = Some(ids);
        self
    }
}

/// The collected violations.
struct Violations(Vec<TypeMapViolation>);

impl Violations {
    /// Add a violation.
    fn add(&mut self, rule: TypeMapRule, position: usize, value: u32) -> Result<()> {
        self.0.try_reserve(1).map_err(|_| no_memory())?;
        self.0.push(TypeMapViolation {
            rule,
            position,
            value,
        });
        Ok(())
    }

    /// Add a violation of `rule` for each value that is not 0 and not in
    /// `first..end`.
    fn values(
        &mut self,
        rule: TypeMapRule,
        values: impl Iterator<Item = u32>,
        end: u32,
    ) -> Result<()> {
        for (position, value) in values.enumerate() {
            if value != 0 && !(FIRST_RECORD_INDEX..end).contains(&value) {
                self.add(rule, position, value)?;
            }
        }
        Ok(())
    }
}

/// Return `true` if the IPI record `id` is a function ID with `function_type`.
fn maps_to(ids: &IdTable<'_>, id: u32, function_type: u32) -> bool {
    match ids.leaf(IdIndex(id)) {
        Ok(IdRecord::Function(value)) => value.function_type.0 == function_type,
        Ok(IdRecord::MemberFunction(value)) => value.function_type.0 == function_type,
        _ => false,
    }
}

impl TypeMapStream<'_> {
    /// Check the map values, the record offsets and the function ID pairs.
    ///
    /// The function returns the violations in stream order.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded)
    /// if an allocation fails.
    pub fn validate(&self, context: &TypeMapContext<'_>) -> Result<Vec<TypeMapViolation>> {
        let mut violations = Violations(Vec::new());
        let type_end = context.type_index_end.0;
        let id_end = context.id_index_end.0;
        // The input index of the first map value, the map with the function
        // IDs, and the number of input function IDs.
        let (first, function_map) = match self.kind() {
            TypeMapKind::TypeServer => {
                violations.values(TypeMapRule::TypeValue, self.types(), type_end)?;
                violations.values(TypeMapRule::IdValue, self.ids(), id_end)?;
                (FIRST_RECORD_INDEX, self.ids)
            }
            TypeMapKind::Object { .. } => {
                let end = type_end.max(id_end);
                violations.values(TypeMapRule::ObjectValue, self.types(), end)?;
                let count = u32::try_from(self.types.len()).ok();
                if self.first_index() < FIRST_RECORD_INDEX
                    || count
                        .and_then(|count| self.first_index().checked_add(count))
                        .is_none()
                {
                    violations.add(TypeMapRule::FirstIndex, 0, self.first_index())?;
                }
                self.record_offsets_rule(&mut violations)?;
                (self.first_index(), self.types)
            }
        };
        for (position, (function, function_type)) in self.function_types().enumerate() {
            let mapped = function
                .checked_sub(first)
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| function_map.get(index))
                .map(|word| u32::from_le_bytes(*word));
            let Some(mapped) = mapped else {
                violations.add(TypeMapRule::FunctionInput, position, function)?;
                continue;
            };
            if !(FIRST_RECORD_INDEX..type_end).contains(&function_type) {
                violations.add(TypeMapRule::FunctionType, position, function_type)?;
                continue;
            }
            if mapped != 0
                && let Some(ids) = context.ids
                && !maps_to(ids, mapped, function_type)
            {
                violations.add(TypeMapRule::FunctionMapping, position, mapped)?;
            }
        }
        Ok(violations.0)
    }

    /// Check that the record offsets start at 0 and grow by whole records.
    fn record_offsets_rule(&self, violations: &mut Violations) -> Result<()> {
        let mut previous = None;
        for (position, offset) in self.record_offsets().enumerate() {
            let valid = match previous {
                None => offset == 0,
                Some(previous) => offset
                    .checked_sub(previous)
                    .is_some_and(|step| step >= MIN_RECORD_SIZE && step.is_multiple_of(4)),
            };
            if !valid {
                violations.add(TypeMapRule::RecordOffset, position, offset)?;
            }
            previous = Some(offset);
        }
        Ok(())
    }
}
