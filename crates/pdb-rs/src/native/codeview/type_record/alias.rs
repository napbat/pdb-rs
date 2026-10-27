//! Type records that name another type or an expression: `LF_ALIAS`,
//! `LF_DEFARG` and `LF_MANAGED`.
//!
//! The layouts follow `lfAlias`, `lfDefArg` and `lfManaged` in `cvinfo.h`.
//! The strings are NUL-terminated, because these leaf kinds are after
//! `LF_ST_MAX`.

use super::fields;
use crate::bytes::into_owned;
use crate::native::codeview::index::TypeIndex;
use crate::native::output::Output;
use crate::{Bytes, Result};

/// An `LF_ALIAS` record: a `typedef` name.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Alias<'a> {
    /// The underlying type.
    pub underlying_type: TypeIndex,
    /// The alias name.
    pub name: Bytes<'a>,
}

impl<'a> Alias<'a> {
    /// Decode the payload of an `LF_ALIAS` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            underlying_type: fields.type_index()?,
            name: Bytes::new(fields.name()?),
        };
        fields.finish_type_padding()?;
        Ok(value)
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.underlying_type)?;
        writer.c_string(&self.name)
    }
}

into_owned!(struct Alias { name; underlying_type });

/// An `LF_DEFARG` record: a default argument value.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DefaultArgument<'a> {
    /// The type of the expression.
    pub argument_type: TypeIndex,
    /// The expression text.
    pub expression: Bytes<'a>,
}

impl<'a> DefaultArgument<'a> {
    /// Decode the payload of an `LF_DEFARG` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            argument_type: fields.type_index()?,
            expression: Bytes::new(fields.name()?),
        };
        fields.finish_type_padding()?;
        Ok(value)
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.argument_type)?;
        writer.c_string(&self.expression)
    }
}

into_owned!(struct DefaultArgument { expression; argument_type });

/// Decode the payload of an `LF_MANAGED` record: the UTF-8 type name.
pub(super) fn decode_managed(data: &[u8]) -> Result<&[u8]> {
    let mut fields = fields(data);
    let name = fields.name()?;
    fields.finish_type_padding()?;
    Ok(name)
}
