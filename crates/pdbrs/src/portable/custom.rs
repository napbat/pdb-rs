//! Custom debug information of Portable PDBs.
//!
//! A `CustomDebugInformation` row has a kind GUID and a value blob. [`CustomDebugValue::parse`]
//! selects the codec from the kind and returns a [`CustomDebugValue`]. A kind
//! that this module does not know gives [`CustomDebugValue::Unknown`] with the
//! raw bytes. The decoder does not drop or interpret such a value.
//!
//! [`Pdb::custom_debug_information`](crate::portable::Pdb::custom_debug_information)
//! reads a row with its parent, its GUID and its blob as a [`CustomDebugInformation`]
//! record, and [`CustomDebugInformation::decode`] decodes it.
//!
//! The layouts come from these sources:
//!
//! - The "Language Specific Custom Debug Information Records" section of
//!   `docs/design/specs/PortablePdb-Metadata.md` in `dotnet/runtime` (commit
//!   `736475ca4a183e931d84cafb4954cff582b59ae9`).
//! - Roslyn (MIT) at commit `e3c31f15b79e226656e3b082a921d4d60127b1fa`:
//!   `src/Dependencies/CodeAnalysis.Debugging/PortableCustomDebugInfoKinds.cs`,
//!   `src/Compilers/Core/Portable/PEWriter/MetadataWriter.PortablePdb.cs` and
//!   `src/Compilers/Core/Portable/Emit/EditAndContinueMethodDebugInformation.cs`.
//!   They define the kinds that the specification does not list: async method
//!   stepping information, tuple element names, the Edit and Continue state
//!   machine state map and primary constructor information.
//!
//! | Kind | Parent | Codec |
//! | --- | --- | --- |
//! | [`CustomDebugKind::ASYNC_METHOD_STEPPING_INFORMATION`] | `MethodDef` | [`AsyncMethodSteppingInformation`] |
//! | [`CustomDebugKind::STATE_MACHINE_HOISTED_LOCAL_SCOPES`] | `MethodDef` | [`HoistedLocalScopes`] |
//! | [`CustomDebugKind::DYNAMIC_LOCAL_VARIABLES`] | `LocalVariable`, `LocalConstant` | [`DynamicLocalVariables`] |
//! | [`CustomDebugKind::TUPLE_ELEMENT_NAMES`] | `LocalVariable`, `LocalConstant` | [`TupleElementNames`] |
//! | [`CustomDebugKind::DEFAULT_NAMESPACE`] | `Module` | [`Bytes`] (UTF-8) |
//! | [`CustomDebugKind::ENC_LOCAL_SLOT_MAP`] | `MethodDef` | [`EncLocalSlotMap`] |
//! | [`CustomDebugKind::ENC_LAMBDA_AND_CLOSURE_MAP`] | `MethodDef` | [`EncLambdaAndClosureMap`] |
//! | [`CustomDebugKind::ENC_STATE_MACHINE_STATE_MAP`] | `MethodDef` | [`EncStateMachineStateMap`] |
//! | [`CustomDebugKind::SOURCE_LINK`] | `Module` | [`Bytes`] (UTF-8 JSON) |
//! | [`CustomDebugKind::EMBEDDED_SOURCE`] | `Document` | [`EmbeddedSource`] |
//! | [`CustomDebugKind::COMPILATION_METADATA_REFERENCES`] | `Module` | [`CompilationMetadataReferences`] |
//! | [`CustomDebugKind::COMPILATION_OPTIONS`] | `Module` | [`CompilationOptions`] |
//! | [`CustomDebugKind::TYPE_DEFINITION_DOCUMENTS`] | `TypeDef` | [`TypeDefinitionDocuments`] |
//! | [`CustomDebugKind::PRIMARY_CONSTRUCTOR_INFORMATION`] | `MethodDef` | An empty blob |
//!
//! Each codec also has an encoder. The encoders write the form that Roslyn
//! writes. The decoders also accept other forms that Roslyn reads, for example
//! compressed integers that are longer than necessary. For such a blob, the
//! encoded bytes are different from the input. Keep the raw bytes to preserve
//! the blob without change.
//!
//! The native `MD2` records of kinds 6, 7 and 9 use the Edit and Continue
//! codecs of this module (see
//! [`crate::native::codeview::symbol::CustomDebugInfoData`]).
//!
//! The codec errors have no location, because a record holds no position. A blob that ends
//! inside a field
//! gives [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof).

/// Define an iterator type over the entries of a blob that its codec checked.
macro_rules! entry_iter {
    ($(#[$doc:meta])* $iter:ident => $item:ty) => {
        $(#[$doc])*
        #[derive(Clone, Debug)]
        pub struct $iter<'a>($crate::portable::custom::Entries<'a, $item>);

        impl<'a> Iterator for $iter<'a> {
            type Item = $item;

            fn next(&mut self) -> Option<Self::Item> {
                self.0.next()
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                self.0.size_hint()
            }
        }

        impl ExactSizeIterator for $iter<'_> {}

        impl core::iter::FusedIterator for $iter<'_> {}
    };
}

/// Define the iterator type `$iter` of a codec type, and implement `IntoIterator` for a
/// reference to the codec type through its `iter` method. The `@into` form only implements
/// `IntoIterator` for an iterator type that the codec module defines. The lifetime `'a` in
/// `$item` is the lifetime of the reference.
macro_rules! iterable {
    ($list:ident => $iter:ident, $item:ty) => {
        entry_iter!(
            #[doc = concat!("An iterator over the entries of a [`", stringify!($list), "`].")]
            $iter => $item
        );

        iterable!(@into $list => $iter, $item);
    };
    (@into $list:ident => $iter:ident, $item:ty) => {
        impl<'a> IntoIterator for &'a $list<'_> {
            type Item = $item;
            type IntoIter = $iter<'a>;

            fn into_iter(self) -> Self::IntoIter {
                self.iter()
            }
        }
    };
}

pub(super) mod blob;
mod compilation;
mod documents;
mod enc;
mod locals;
mod rows;
mod state_machine;

use alloc::vec::Vec;
use core::fmt;
use core::iter::FusedIterator;

pub use compilation::{
    CompilationMetadataReferences, CompilationMetadataReferencesIter, CompilationOption,
    CompilationOptions, CompilationOptionsIter, MetadataReference,
};
pub use documents::{TypeDefinitionDocuments, TypeDefinitionDocumentsIter};
pub use enc::{
    Closures, EncLambdaAndClosureMap, EncLocalSlotMap, EncLocalSlotMapIter,
    EncStateMachineStateMap, EncStateMachineStateMapIter, Lambda, LambdaClosure, Lambdas,
    LocalSlot, MAX_LOCAL_SLOT_KIND, StateMachineState,
};
pub use locals::{
    DynamicLocalVariables, DynamicLocalVariablesIter, TupleElementNames, TupleElementNamesIter,
};
pub use rows::{CustomDebugHandle, CustomDebugInformation, CustomDebugParent};
pub use state_machine::{
    AsyncMethodSteppingInformation, AsyncMethodSteppingInformationIter, AsyncStep,
    HoistedLocalScope, HoistedLocalScopes, HoistedLocalScopesIter,
};

use self::blob::{Reader, Writer, invalid};
use crate::Result;
use crate::bytes::Bytes;
use crate::portable::embedded::EmbeddedSource;

/// The kind of a custom debug information value: a GUID.
///
/// The bytes use the serialized order of the `#GUID` heap: the first three
/// fields are little-endian. The text form `6da9a61e-f8c7-...` uses another order.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CustomDebugKind(pub [u8; 16]);

impl CustomDebugKind {
    /// `54fd2ac5-e925-401a-9c2a-f94f171072f8`: async method stepping information.
    pub const ASYNC_METHOD_STEPPING_INFORMATION: Self = Self::from_fields(
        0x54fd_2ac5,
        0xe925,
        0x401a,
        [0x9c, 0x2a, 0xf9, 0x4f, 0x17, 0x10, 0x72, 0xf8],
    );
    /// `6da9a61e-f8c7-4874-be62-68bc5630df71`: state machine hoisted local scopes.
    pub const STATE_MACHINE_HOISTED_LOCAL_SCOPES: Self = Self::from_fields(
        0x6da9_a61e,
        0xf8c7,
        0x4874,
        [0xbe, 0x62, 0x68, 0xbc, 0x56, 0x30, 0xdf, 0x71],
    );
    /// `83c563c4-b4f3-47d5-b824-ba5441477ea8`: dynamic local variables.
    pub const DYNAMIC_LOCAL_VARIABLES: Self = Self::from_fields(
        0x83c5_63c4,
        0xb4f3,
        0x47d5,
        [0xb8, 0x24, 0xba, 0x54, 0x41, 0x47, 0x7e, 0xa8],
    );
    /// `ed9fdf71-8879-4747-8ed3-fe5ede3ce710`: tuple element names.
    pub const TUPLE_ELEMENT_NAMES: Self = Self::from_fields(
        0xed9f_df71,
        0x8879,
        0x4747,
        [0x8e, 0xd3, 0xfe, 0x5e, 0xde, 0x3c, 0xe7, 0x10],
    );
    /// `58b2eab6-209f-4e4e-a22c-b2d0f910c782`: the default namespace (VB).
    pub const DEFAULT_NAMESPACE: Self = Self::from_fields(
        0x58b2_eab6,
        0x209f,
        0x4e4e,
        [0xa2, 0x2c, 0xb2, 0xd0, 0xf9, 0x10, 0xc7, 0x82],
    );
    /// `755f52a8-91c5-45be-b4b8-209571e552bd`: the Edit and Continue local slot map.
    pub const ENC_LOCAL_SLOT_MAP: Self = Self::from_fields(
        0x755f_52a8,
        0x91c5,
        0x45be,
        [0xb4, 0xb8, 0x20, 0x95, 0x71, 0xe5, 0x52, 0xbd],
    );
    /// `a643004c-0240-496f-a783-30d64f4979de`: the Edit and Continue lambda
    /// and closure map.
    pub const ENC_LAMBDA_AND_CLOSURE_MAP: Self = Self::from_fields(
        0xa643_004c,
        0x0240,
        0x496f,
        [0xa7, 0x83, 0x30, 0xd6, 0x4f, 0x49, 0x79, 0xde],
    );
    /// `8b78cd68-2ede-420b-980b-e15884b8aaa3`: the Edit and Continue state
    /// machine state map.
    pub const ENC_STATE_MACHINE_STATE_MAP: Self = Self::from_fields(
        0x8b78_cd68,
        0x2ede,
        0x420b,
        [0x98, 0x0b, 0xe1, 0x58, 0x84, 0xb8, 0xaa, 0xa3],
    );
    /// `cc110556-a091-4d38-9fec-25ab9a351a6a`: Source Link JSON.
    pub const SOURCE_LINK: Self = Self::from_fields(
        0xcc11_0556,
        0xa091,
        0x4d38,
        [0x9f, 0xec, 0x25, 0xab, 0x9a, 0x35, 0x1a, 0x6a],
    );
    /// `0e8a571b-6926-466e-b4ad-8ab04611f5fe`: embedded source.
    pub const EMBEDDED_SOURCE: Self = Self::from_fields(
        0x0e8a_571b,
        0x6926,
        0x466e,
        [0xb4, 0xad, 0x8a, 0xb0, 0x46, 0x11, 0xf5, 0xfe],
    );
    /// `7e4d4708-096e-4c5c-aeda-cb10ba6a740d`: compilation metadata references.
    pub const COMPILATION_METADATA_REFERENCES: Self = Self::from_fields(
        0x7e4d_4708,
        0x096e,
        0x4c5c,
        [0xae, 0xda, 0xcb, 0x10, 0xba, 0x6a, 0x74, 0x0d],
    );
    /// `b5feec05-8cd0-4a83-96da-466284bb4bd8`: compilation options.
    pub const COMPILATION_OPTIONS: Self = Self::from_fields(
        0xb5fe_ec05,
        0x8cd0,
        0x4a83,
        [0x96, 0xda, 0x46, 0x62, 0x84, 0xbb, 0x4b, 0xd8],
    );
    /// `932e74bc-dba9-4478-8d46-0f32a7bab3d3`: type definition documents.
    pub const TYPE_DEFINITION_DOCUMENTS: Self = Self::from_fields(
        0x932e_74bc,
        0xdba9,
        0x4478,
        [0x8d, 0x46, 0x0f, 0x32, 0xa7, 0xba, 0xb3, 0xd3],
    );
    /// `9d40ace1-c703-4d0e-bf41-7243060a8fb5`: primary constructor information.
    pub const PRIMARY_CONSTRUCTOR_INFORMATION: Self = Self::from_fields(
        0x9d40_ace1,
        0xc703,
        0x4d0e,
        [0xbf, 0x41, 0x72, 0x43, 0x06, 0x0a, 0x8f, 0xb5],
    );

    /// Make a kind from the fields of the GUID text form.
    #[must_use]
    pub const fn from_fields(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
        let [a0, a1, a2, a3] = data1.to_le_bytes();
        let [b0, b1] = data2.to_le_bytes();
        let [c0, c1] = data3.to_le_bytes();
        let [d0, d1, d2, d3, d4, d5, d6, d7] = data4;
        Self([
            a0, a1, a2, a3, b0, b1, c0, c1, d0, d1, d2, d3, d4, d5, d6, d7,
        ])
    }

    /// Return the GUID bytes in their serialized order.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for CustomDebugKind {
    /// Write the GUID as lowercase text without braces, for example
    /// `6da9a61e-f8c7-4874-be62-68bc5630df71`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = &self.0;
        write!(
            f,
            "{:08x}-{:04x}-{:04x}-",
            u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            u16::from_le_bytes([b[4], b[5]]),
            u16::from_le_bytes([b[6], b[7]])
        )?;
        for (index, byte) in b[8..].iter().enumerate() {
            if index == 2 {
                f.write_str("-")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// The reader of one entry. The `i32` is the syntax offset baseline of an
/// Edit and Continue map; the other codecs ignore it.
type Read<'a, T> = fn(&mut Reader<'a>, i32) -> Result<T>;

/// An iterator over the entries of a blob that its codec checked. The public iterator type of
/// each codec wraps it.
#[derive(Clone, Debug)]
struct Entries<'a, T> {
    /// The remaining entries.
    reader: Reader<'a>,
    /// The number of remaining entries.
    remaining: usize,
    /// The syntax offset baseline for `read`.
    baseline: i32,
    /// The reader of one entry.
    read: Read<'a, T>,
}

impl<'a, T> Entries<'a, T> {
    /// Iterate over `count` entries in `bytes`.
    const fn new(bytes: &'a [u8], count: usize, baseline: i32, read: Read<'a, T>) -> Self {
        Self {
            reader: Reader::new(bytes),
            remaining: count,
            baseline,
            read,
        }
    }
}

impl<T> Iterator for Entries<'_, T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        if self.remaining == 0 {
            return None;
        }
        // The codec checked all entries, so this read cannot fail.
        let entry = (self.read)(&mut self.reader, self.baseline).ok();
        self.remaining = if entry.is_some() {
            self.remaining - 1
        } else {
            0
        };
        entry
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T> ExactSizeIterator for Entries<'_, T> {}

impl<T> FusedIterator for Entries<'_, T> {}

/// Read entries with `read` to the end of `bytes`, and return their number.
fn count_to_end<'a, T>(bytes: &'a [u8], baseline: i32, read: Read<'a, T>) -> Result<usize> {
    let mut reader = Reader::new(bytes);
    let mut count = 0;
    while !reader.is_empty() {
        read(&mut reader, baseline)?;
        count += 1;
    }
    Ok(count)
}

/// A decoded custom debug information value. It borrows the blob until
/// [`CustomDebugValue::into_owned`] copies it.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum CustomDebugValue<'a> {
    /// [`CustomDebugKind::ASYNC_METHOD_STEPPING_INFORMATION`].
    AsyncMethodSteppingInformation(AsyncMethodSteppingInformation<'a>),
    /// [`CustomDebugKind::STATE_MACHINE_HOISTED_LOCAL_SCOPES`].
    StateMachineHoistedLocalScopes(HoistedLocalScopes<'a>),
    /// [`CustomDebugKind::DYNAMIC_LOCAL_VARIABLES`].
    DynamicLocalVariables(DynamicLocalVariables<'a>),
    /// [`CustomDebugKind::TUPLE_ELEMENT_NAMES`].
    TupleElementNames(TupleElementNames<'a>),
    /// [`CustomDebugKind::DEFAULT_NAMESPACE`]: the namespace name.
    DefaultNamespace(Bytes<'a>),
    /// [`CustomDebugKind::ENC_LOCAL_SLOT_MAP`].
    EncLocalSlotMap(EncLocalSlotMap<'a>),
    /// [`CustomDebugKind::ENC_LAMBDA_AND_CLOSURE_MAP`].
    EncLambdaAndClosureMap(EncLambdaAndClosureMap<'a>),
    /// [`CustomDebugKind::ENC_STATE_MACHINE_STATE_MAP`].
    EncStateMachineStateMap(EncStateMachineStateMap<'a>),
    /// [`CustomDebugKind::SOURCE_LINK`]: the JSON text. pdbrs does not parse
    /// the JSON and does not fetch the URLs.
    SourceLink(Bytes<'a>),
    /// [`CustomDebugKind::EMBEDDED_SOURCE`].
    EmbeddedSource(EmbeddedSource<'a>),
    /// [`CustomDebugKind::COMPILATION_METADATA_REFERENCES`].
    CompilationMetadataReferences(CompilationMetadataReferences<'a>),
    /// [`CustomDebugKind::COMPILATION_OPTIONS`].
    CompilationOptions(CompilationOptions<'a>),
    /// [`CustomDebugKind::TYPE_DEFINITION_DOCUMENTS`].
    TypeDefinitionDocuments(TypeDefinitionDocuments<'a>),
    /// [`CustomDebugKind::PRIMARY_CONSTRUCTOR_INFORMATION`]: the method is a
    /// primary constructor. The blob is empty.
    PrimaryConstructorInformation,
    /// A kind that this module does not decode, with the raw blob.
    Unknown {
        /// The kind GUID.
        kind: CustomDebugKind,
        /// The raw blob.
        value: Bytes<'a>,
    },
}

impl<'a> CustomDebugValue<'a> {
    /// Decode the blob of a custom debug information value of `kind`.
    ///
    /// # Errors
    ///
    /// Return the error of the codec of `kind` if the blob is not valid. An
    /// unknown kind does not give an error.
    pub fn parse(kind: CustomDebugKind, blob: &'a [u8]) -> Result<Self> {
        Ok(match kind {
            CustomDebugKind::ASYNC_METHOD_STEPPING_INFORMATION => {
                CustomDebugValue::AsyncMethodSteppingInformation(
                    AsyncMethodSteppingInformation::parse(blob)?,
                )
            }
            CustomDebugKind::STATE_MACHINE_HOISTED_LOCAL_SCOPES => {
                CustomDebugValue::StateMachineHoistedLocalScopes(HoistedLocalScopes::parse(blob)?)
            }
            CustomDebugKind::DYNAMIC_LOCAL_VARIABLES => {
                CustomDebugValue::DynamicLocalVariables(DynamicLocalVariables::parse(blob))
            }
            CustomDebugKind::TUPLE_ELEMENT_NAMES => {
                CustomDebugValue::TupleElementNames(TupleElementNames::parse(blob)?)
            }
            CustomDebugKind::DEFAULT_NAMESPACE => {
                CustomDebugValue::DefaultNamespace(Bytes::new(blob))
            }
            CustomDebugKind::ENC_LOCAL_SLOT_MAP => {
                CustomDebugValue::EncLocalSlotMap(EncLocalSlotMap::parse(blob)?)
            }
            CustomDebugKind::ENC_LAMBDA_AND_CLOSURE_MAP => {
                CustomDebugValue::EncLambdaAndClosureMap(EncLambdaAndClosureMap::parse(blob)?)
            }
            CustomDebugKind::ENC_STATE_MACHINE_STATE_MAP => {
                CustomDebugValue::EncStateMachineStateMap(EncStateMachineStateMap::parse(blob)?)
            }
            CustomDebugKind::SOURCE_LINK => CustomDebugValue::SourceLink(Bytes::new(blob)),
            CustomDebugKind::EMBEDDED_SOURCE => {
                CustomDebugValue::EmbeddedSource(EmbeddedSource::parse(blob)?)
            }
            CustomDebugKind::COMPILATION_METADATA_REFERENCES => {
                CustomDebugValue::CompilationMetadataReferences(
                    CompilationMetadataReferences::parse(blob)?,
                )
            }
            CustomDebugKind::COMPILATION_OPTIONS => {
                CustomDebugValue::CompilationOptions(CompilationOptions::parse(blob)?)
            }
            CustomDebugKind::TYPE_DEFINITION_DOCUMENTS => {
                CustomDebugValue::TypeDefinitionDocuments(TypeDefinitionDocuments::parse(blob)?)
            }
            CustomDebugKind::PRIMARY_CONSTRUCTOR_INFORMATION => {
                if !blob.is_empty() {
                    return Err(invalid("primary constructor information is not empty"));
                }
                CustomDebugValue::PrimaryConstructorInformation
            }
            _ => CustomDebugValue::Unknown {
                kind,
                value: Bytes::new(blob),
            },
        })
    }

    /// Return a record that owns its bytes. Copy borrowed bytes.
    #[must_use]
    pub fn into_owned(self) -> CustomDebugValue<'static> {
        match self {
            Self::AsyncMethodSteppingInformation(v) => {
                CustomDebugValue::AsyncMethodSteppingInformation(v.into_owned())
            }
            Self::StateMachineHoistedLocalScopes(v) => {
                CustomDebugValue::StateMachineHoistedLocalScopes(v.into_owned())
            }
            Self::DynamicLocalVariables(v) => {
                CustomDebugValue::DynamicLocalVariables(v.into_owned())
            }
            Self::TupleElementNames(v) => CustomDebugValue::TupleElementNames(v.into_owned()),
            Self::DefaultNamespace(v) => CustomDebugValue::DefaultNamespace(v.into_owned()),
            Self::EncLocalSlotMap(v) => CustomDebugValue::EncLocalSlotMap(v.into_owned()),
            Self::EncLambdaAndClosureMap(v) => {
                CustomDebugValue::EncLambdaAndClosureMap(v.into_owned())
            }
            Self::EncStateMachineStateMap(v) => {
                CustomDebugValue::EncStateMachineStateMap(v.into_owned())
            }
            Self::SourceLink(v) => CustomDebugValue::SourceLink(v.into_owned()),
            Self::EmbeddedSource(v) => CustomDebugValue::EmbeddedSource(v.into_owned()),
            Self::CompilationMetadataReferences(v) => {
                CustomDebugValue::CompilationMetadataReferences(v.into_owned())
            }
            Self::CompilationOptions(v) => CustomDebugValue::CompilationOptions(v.into_owned()),
            Self::TypeDefinitionDocuments(v) => {
                CustomDebugValue::TypeDefinitionDocuments(v.into_owned())
            }
            Self::PrimaryConstructorInformation => CustomDebugValue::PrimaryConstructorInformation,
            Self::Unknown { kind, value } => CustomDebugValue::Unknown {
                kind,
                value: value.into_owned(),
            },
        }
    }

    /// Return the kind GUID of the value.
    #[must_use]
    pub const fn kind(&self) -> CustomDebugKind {
        match self {
            Self::AsyncMethodSteppingInformation(_) => {
                CustomDebugKind::ASYNC_METHOD_STEPPING_INFORMATION
            }
            Self::StateMachineHoistedLocalScopes(_) => {
                CustomDebugKind::STATE_MACHINE_HOISTED_LOCAL_SCOPES
            }
            Self::DynamicLocalVariables(_) => CustomDebugKind::DYNAMIC_LOCAL_VARIABLES,
            Self::TupleElementNames(_) => CustomDebugKind::TUPLE_ELEMENT_NAMES,
            Self::DefaultNamespace(_) => CustomDebugKind::DEFAULT_NAMESPACE,
            Self::EncLocalSlotMap(_) => CustomDebugKind::ENC_LOCAL_SLOT_MAP,
            Self::EncLambdaAndClosureMap(_) => CustomDebugKind::ENC_LAMBDA_AND_CLOSURE_MAP,
            Self::EncStateMachineStateMap(_) => CustomDebugKind::ENC_STATE_MACHINE_STATE_MAP,
            Self::SourceLink(_) => CustomDebugKind::SOURCE_LINK,
            Self::EmbeddedSource(_) => CustomDebugKind::EMBEDDED_SOURCE,
            Self::CompilationMetadataReferences(_) => {
                CustomDebugKind::COMPILATION_METADATA_REFERENCES
            }
            Self::CompilationOptions(_) => CustomDebugKind::COMPILATION_OPTIONS,
            Self::TypeDefinitionDocuments(_) => CustomDebugKind::TYPE_DEFINITION_DOCUMENTS,
            Self::PrimaryConstructorInformation => CustomDebugKind::PRIMARY_CONSTRUCTOR_INFORMATION,
            Self::Unknown { kind, .. } => *kind,
        }
    }

    /// Encode the value again from its decoded entries, and append it to `out`.
    ///
    /// The result has the form that Roslyn writes. An unknown value, text and
    /// an embedded source give their bytes without change. The encoders drop
    /// the bytes that a decoder keeps but does not interpret, for example
    /// [`EncStateMachineStateMap::trailing`].
    ///
    /// # Errors
    ///
    /// Return [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if
    /// an allocation fails. A decoded value always has an encoding. On error,
    /// `out` does not change.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        match self {
            Self::AsyncMethodSteppingInformation(info) => {
                AsyncMethodSteppingInformation::encode_parts(
                    info.catch_handler_offset(),
                    info.iter(),
                    out,
                )
            }
            Self::StateMachineHoistedLocalScopes(scopes) => {
                HoistedLocalScopes::encode_parts(scopes.iter(), out)
            }
            Self::DynamicLocalVariables(flags) => {
                DynamicLocalVariables::encode_parts(flags.iter(), out)
            }
            Self::TupleElementNames(names) => TupleElementNames::encode_parts(names.iter(), out),
            Self::EncLocalSlotMap(map) => EncLocalSlotMap::encode_parts(map.iter(), out),
            Self::EncLambdaAndClosureMap(map) => EncLambdaAndClosureMap::encode_parts(
                map.method_ordinal(),
                map.closures(),
                map.lambdas(),
                out,
            ),
            Self::EncStateMachineStateMap(map) => {
                EncStateMachineStateMap::encode_parts(map.iter(), out)
            }
            Self::DefaultNamespace(text) | Self::SourceLink(text) => copy(text, out),
            Self::EmbeddedSource(source) => source.encode(out),
            Self::CompilationMetadataReferences(references) => {
                CompilationMetadataReferences::encode_parts(references.iter(), out)
            }
            Self::CompilationOptions(options) => {
                CompilationOptions::encode_parts(options.iter(), out)
            }
            Self::TypeDefinitionDocuments(documents) => {
                TypeDefinitionDocuments::encode_parts(documents.iter(), out)
            }
            Self::PrimaryConstructorInformation => Ok(()),
            Self::Unknown { value, .. } => copy(value, out),
        }
    }
}

impl crate::bytes::IntoOwned for CustomDebugValue<'_> {
    type Owned = CustomDebugValue<'static>;

    fn to_static(self) -> Self::Owned {
        self.into_owned()
    }
}

/// Append `bytes` to `out`.
fn copy(bytes: &[u8], out: &mut Vec<u8>) -> Result<()> {
    let mut writer = Writer::append(out);
    writer.bytes(bytes)?;
    writer.finish();
    Ok(())
}
