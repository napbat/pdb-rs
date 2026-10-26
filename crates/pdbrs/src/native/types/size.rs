//! The size in bytes of a type.
//!
//! [`type_size`] follows the type records to a record that stores a size:
//!
//! - A primitive type has the size of its kind, or the size of its pointer
//!   mode ([`Primitive::size`](crate::native::codeview::index::Primitive::size)).
//! - `LF_POINTER` stores the size in its attributes.
//! - `LF_ARRAY`, `LF_STRIDED_ARRAY`, `LF_VECTOR`, `LF_MATRIX` and the UDT
//!   kinds store the size as a numeric leaf.
//! - `LF_MODIFIER`, `LF_ALIAS`, `LF_BITFIELD` and `LF_ENUM` have the size of
//!   the type that they refer to.
//! - A UDT forward reference has the size of its definition, which
//!   [`ForwardRefs`] finds.
//!
//! Other kinds, for example procedure types, have no size.

use super::{ForwardRefs, TypeTable};
use crate::Result;
use crate::native::codeview::index::{Primitive, TypeIndex};
use crate::native::codeview::numeric::{Integer, Numeric};
use crate::native::codeview::type_record::TypeLeaf;

/// The largest number of records that [`type_size`] follows.
///
/// A longer chain is a reference cycle or a hostile stream.
const MAX_STEPS: usize = 64;

/// Return the value of a numeric size, or `None` if it is not an unsigned
/// integer that fits in `u64`.
fn numeric_size(size: &Numeric<'_>) -> Option<u64> {
    size.integer().and_then(Integer::to_u64)
}

/// Return the size in bytes of the type at `index`.
///
/// `forward` resolves UDT forward references. Without it, a forward
/// reference has no known size.
///
/// The function returns `Ok(None)` if the type has no size, if a record
/// that it follows is not in `table`, if the size is 0 or does not fit in
/// `u64`, or if the chain of records is longer than 64 records.
///
/// # Errors
///
/// Returns the decoding errors of the records that the function follows,
/// and the errors of [`ForwardRefs::resolve`].
pub fn type_size(
    table: &TypeTable<'_>,
    forward: Option<&ForwardRefs<'_, '_>>,
    index: TypeIndex,
) -> Result<Option<u64>> {
    let mut index = index;
    for _ in 0..MAX_STEPS {
        if index.is_primitive() {
            let primitive = index.primitive().ok().flatten();
            return Ok(primitive
                .and_then(Primitive::size)
                .map(u64::from)
                .filter(|&size| size != 0));
        }
        let Ok(leaf) = table.leaf(index) else {
            return Ok(None);
        };
        if let Some(tag) = leaf.tag()
            && tag.options.is_forward_reference()
        {
            match forward.map(|forward| forward.resolve(index)).transpose()? {
                Some(Some(definition)) if definition != index => {
                    index = definition;
                    continue;
                }
                _ => return Ok(None),
            }
        }
        let size = match &leaf {
            TypeLeaf::Pointer(pointer) => Some(u64::from(pointer.attributes.size())),
            TypeLeaf::Array(array) => numeric_size(&array.size),
            TypeLeaf::StridedArray(array) => numeric_size(&array.size),
            TypeLeaf::Vector(vector) => numeric_size(&vector.size),
            TypeLeaf::Matrix(matrix) => numeric_size(&matrix.size),
            TypeLeaf::Class(value) => numeric_size(&value.size),
            TypeLeaf::WideClass(value) => numeric_size(&value.size),
            TypeLeaf::Union(value) => numeric_size(&value.size),
            TypeLeaf::WideUnion(value) => numeric_size(&value.size),
            TypeLeaf::Modifier(value) => {
                index = value.modified_type;
                continue;
            }
            TypeLeaf::Alias(value) => {
                index = value.underlying_type;
                continue;
            }
            TypeLeaf::Bitfield(value) => {
                index = value.underlying_type;
                continue;
            }
            TypeLeaf::Enum(value) => {
                index = value.underlying_type;
                continue;
            }
            _ => None,
        };
        return Ok(size.filter(|&size| size != 0));
    }
    Ok(None)
}
