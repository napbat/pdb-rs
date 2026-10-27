//! The Edit and Continue lambda and closure map.
//!
//! The blob holds compressed unsigned integers:
//!
//! 1. The method ordinal plus 1.
//! 2. The negated syntax offset baseline.
//! 3. The closure count, and the syntax offset of each closure.
//! 4. To the end of the blob: the syntax offset and the closure ordinal of
//!    each lambda.
//!
//! Roslyn stores the closure ordinal plus 2: 0 for a lambda that captures only
//! `this`, 1 for a static lambda and `2 + index` for a closure index. The text
//! of the specification (0 for no closure, then a 1-based index) does not
//! agree with Roslyn; this codec follows Roslyn.

use alloc::vec::Vec;

use super::super::Entries;
use super::super::blob::{Reader, Writer, invalid, too_large};
use super::{read_baseline, read_syntax_offset, write_baseline, write_syntax_offset};
use crate::Result;
use crate::bytes::{Bytes, into_owned};

/// The baseline when no syntax offset is smaller than -1.
const DEFAULT_BASELINE: i32 = -1;

/// The closure of a lambda.
///
/// The default value is [`LambdaClosure::ThisOnly`], the value of the stored
/// ordinal 0.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum LambdaClosure {
    /// The lambda captures only `this`.
    #[default]
    ThisOnly,
    /// The lambda captures nothing.
    Static,
    /// The lambda uses the closure with this 0-based index.
    Closure(u32),
}

/// One lambda of an Edit and Continue lambda and closure map.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Lambda {
    /// The syntax offset of the lambda.
    pub syntax_offset: i32,
    /// The closure of the lambda.
    pub closure: LambdaClosure,
}

entry_iter!(
    /// An iterator over the syntax offsets of the closures of an
    /// [`EncLambdaAndClosureMap`].
    Closures => i32
);

entry_iter!(
    /// An iterator over the lambdas of an [`EncLambdaAndClosureMap`].
    Lambdas => Lambda
);

/// An Edit and Continue lambda and closure map.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct EncLambdaAndClosureMap<'a> {
    /// The blob.
    bytes: Bytes<'a>,
    /// The method ordinal.
    method_ordinal: i32,
    /// The syntax offset baseline.
    baseline: i32,
    /// The blob offset of the closures. [`EncLambdaAndClosureMap::parse`] checked them.
    closures: usize,
    /// The number of closures.
    closure_count: usize,
    /// The blob offset of the lambdas, to the end of the blob.
    /// [`EncLambdaAndClosureMap::parse`] checked them.
    lambdas: usize,
    /// The number of lambdas.
    lambda_count: usize,
}

into_owned!(struct EncLambdaAndClosureMap {
    bytes;
    method_ordinal, baseline, closures, closure_count, lambdas, lambda_count
});

impl<'a> EncLambdaAndClosureMap<'a> {
    /// Decode a lambda and closure map blob.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof) if the
    ///   blob ends inside a field.
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   compressed integer is not valid, or if the closure index of a lambda
    ///   is not smaller than the closure count.
    pub fn parse(blob: &'a [u8]) -> Result<Self> {
        let mut reader = Reader::new(blob);
        // A compressed integer has at most 29 bits, so the difference fits.
        let method_ordinal = reader.unsigned()?.cast_signed() - 1;
        let baseline = read_baseline(&mut reader)?;
        let declared = reader.unsigned()?;
        let closures = blob.len() - reader.rest().len();
        let mut closure_count = 0;
        for _ in 0..declared {
            read_closure(&mut reader, baseline)?;
            closure_count += 1;
        }
        let lambdas = blob.len() - reader.rest().len();
        let mut lambda_count = 0;
        while !reader.is_empty() {
            let lambda = read_lambda(&mut reader, baseline)?;
            if let LambdaClosure::Closure(index) = lambda.closure
                && index >= declared
            {
                return Err(invalid(
                    "lambda closure index is not below the closure count",
                ));
            }
            lambda_count += 1;
        }
        Ok(Self {
            bytes: Bytes::new(blob),
            method_ordinal,
            baseline,
            closures,
            closure_count,
            lambdas,
            lambda_count,
        })
    }

    /// Return the blob bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return the method ordinal: a number that Roslyn derives from the source
    /// location of the method, or -1 if it is not defined.
    #[must_use]
    pub const fn method_ordinal(&self) -> i32 {
        self.method_ordinal
    }

    /// Return the syntax offset baseline.
    #[must_use]
    pub const fn syntax_offset_baseline(&self) -> i32 {
        self.baseline
    }

    /// Return an iterator over the syntax offsets of the closures.
    #[must_use]
    pub fn closures(&self) -> Closures<'_> {
        Closures(Entries::new(
            &self.bytes[self.closures..self.lambdas],
            self.closure_count,
            self.baseline,
            read_closure,
        ))
    }

    /// Return an iterator over the lambdas.
    #[must_use]
    pub fn lambdas(&self) -> Lambdas<'_> {
        Lambdas(Entries::new(
            &self.bytes[self.lambdas..],
            self.lambda_count,
            self.baseline,
            read_lambda,
        ))
    }

    /// Encode a lambda and closure map blob as Roslyn does, and append it to
    /// `out`.
    ///
    /// The baseline is the smallest of -1 and all syntax offsets.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if the
    ///   method ordinal is smaller than -1, or if the closure index of a
    ///   lambda is not smaller than the closure count.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if a
    ///   value does not fit in a compressed integer, or if an allocation fails.
    ///
    /// On error, `out` does not change.
    pub fn encode_parts(
        method_ordinal: i32,
        closures: impl IntoIterator<Item = i32, IntoIter: Clone>,
        lambdas: impl IntoIterator<Item = Lambda, IntoIter: Clone>,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        if method_ordinal < -1 {
            return Err(invalid("lambda map method ordinal is below -1"));
        }
        let closures = closures.into_iter();
        let lambdas = lambdas.into_iter();
        let baseline = closures
            .clone()
            .chain(lambdas.clone().map(|lambda| lambda.syntax_offset))
            .fold(DEFAULT_BASELINE, i32::min);
        let count = u32::try_from(closures.clone().count())
            .map_err(|_| too_large("lambda map has too many closures"))?;
        let mut writer = Writer::append(out);
        writer.unsigned_wide(i64::from(method_ordinal) + 1)?;
        write_baseline(&mut writer, baseline)?;
        writer.unsigned(count)?;
        for syntax_offset in closures {
            write_syntax_offset(&mut writer, syntax_offset, baseline)?;
        }
        for lambda in lambdas {
            let ordinal = match lambda.closure {
                LambdaClosure::ThisOnly => 0,
                LambdaClosure::Static => 1,
                LambdaClosure::Closure(index) if index < count => index + 2,
                LambdaClosure::Closure(_) => {
                    return Err(invalid(
                        "lambda closure index is not below the closure count",
                    ));
                }
            };
            write_syntax_offset(&mut writer, lambda.syntax_offset, baseline)?;
            writer.unsigned(ordinal)?;
        }
        writer.finish();
        Ok(())
    }
}

/// Read the syntax offset of one closure.
fn read_closure(reader: &mut Reader<'_>, baseline: i32) -> Result<i32> {
    read_syntax_offset(reader, baseline)
}

/// Read one lambda.
fn read_lambda(reader: &mut Reader<'_>, baseline: i32) -> Result<Lambda> {
    let syntax_offset = read_syntax_offset(reader, baseline)?;
    let closure = match reader.unsigned()? {
        0 => LambdaClosure::ThisOnly,
        1 => LambdaClosure::Static,
        stored => LambdaClosure::Closure(stored - 2),
    };
    Ok(Lambda {
        syntax_offset,
        closure,
    })
}
