//! The byte string type of the decoded records.
//!
//! A decoder gives records that borrow their bytes from the input. An editor
//! keeps records that own their bytes. [`Bytes`] holds either form, so that one
//! record type serves both uses.

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::borrow::Borrow;
use core::fmt;
use core::ops::Deref;
use core::str::Utf8Error;

/// Bytes that a record borrows from its input or owns.
///
/// Names stay bytes, because they are not always text. Use [`Bytes::to_str`]
/// to decode them as UTF-8. The `Debug` output is an escaped byte string, for
/// example `b"main"`.
///
/// # Example
///
/// ```
/// use pdbrs::Bytes;
///
/// let name = Bytes::new(b"main");
/// assert_eq!(name, *b"main");
/// assert_eq!(name.to_str(), Ok("main"));
/// assert_eq!(format!("{name:?}"), "b\"main\"");
/// let owned: Bytes<'static> = name.into_owned();
/// assert_eq!(owned.as_borrowed(), None);
/// ```
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Bytes<'a>(Cow<'a, [u8]>);

impl<'a> Bytes<'a> {
    /// Make a value that borrows `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self(Cow::Borrowed(bytes))
    }

    /// Return the bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Return the borrowed bytes with the lifetime of the input.
    ///
    /// Return `None` if the value owns its bytes. A decoder always makes
    /// values that borrow their bytes.
    #[must_use]
    pub const fn as_borrowed(&self) -> Option<&'a [u8]> {
        match self.0 {
            Cow::Borrowed(bytes) => Some(bytes),
            Cow::Owned(_) => None,
        }
    }

    /// Decode the bytes as UTF-8 text.
    ///
    /// # Errors
    ///
    /// Returns the UTF-8 error if the bytes are not UTF-8.
    pub fn to_str(&self) -> Result<&str, Utf8Error> {
        core::str::from_utf8(&self.0)
    }

    /// Return a value that owns its bytes. Copy borrowed bytes.
    #[must_use]
    pub fn into_owned(self) -> Bytes<'static> {
        Bytes(Cow::Owned(self.0.into_owned()))
    }

    /// Return the bytes as a vector. Copy borrowed bytes.
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.0.into_owned()
    }
}

impl Default for Bytes<'_> {
    fn default() -> Self {
        Self::new(&[])
    }
}

impl fmt::Debug for Bytes<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("b\"")?;
        for &byte in self.0.iter() {
            fmt::Display::fmt(&byte.escape_ascii(), f)?;
        }
        f.write_str("\"")
    }
}

impl Deref for Bytes<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for Bytes<'_> {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Borrow<[u8]> for Bytes<'_> {
    fn borrow(&self) -> &[u8] {
        &self.0
    }
}

impl PartialEq<[u8]> for Bytes<'_> {
    fn eq(&self, other: &[u8]) -> bool {
        *self.0 == *other
    }
}

impl PartialEq<&[u8]> for Bytes<'_> {
    fn eq(&self, other: &&[u8]) -> bool {
        *self.0 == **other
    }
}

impl<const N: usize> PartialEq<[u8; N]> for Bytes<'_> {
    fn eq(&self, other: &[u8; N]) -> bool {
        *self.0 == other[..]
    }
}

impl<const N: usize> PartialEq<&[u8; N]> for Bytes<'_> {
    fn eq(&self, other: &&[u8; N]) -> bool {
        *self.0 == other[..]
    }
}

impl PartialEq<Vec<u8>> for Bytes<'_> {
    fn eq(&self, other: &Vec<u8>) -> bool {
        *self.0 == other[..]
    }
}

impl<'a> From<&'a [u8]> for Bytes<'a> {
    fn from(bytes: &'a [u8]) -> Self {
        Self::new(bytes)
    }
}

impl<'a, const N: usize> From<&'a [u8; N]> for Bytes<'a> {
    fn from(bytes: &'a [u8; N]) -> Self {
        Self::new(bytes)
    }
}

impl<'a> From<&'a str> for Bytes<'a> {
    fn from(text: &'a str) -> Self {
        Self::new(text.as_bytes())
    }
}

impl From<Vec<u8>> for Bytes<'_> {
    fn from(bytes: Vec<u8>) -> Self {
        Self(Cow::Owned(bytes))
    }
}

impl From<String> for Bytes<'_> {
    fn from(text: String) -> Self {
        Self(Cow::Owned(text.into_bytes()))
    }
}

impl<'a> From<Cow<'a, [u8]>> for Bytes<'a> {
    fn from(bytes: Cow<'a, [u8]>) -> Self {
        Self(bytes)
    }
}

/// A value that can give a copy that owns all its bytes.
///
/// Each record type with a lifetime implements it through [`into_owned!`].
/// The `into_owned` functions of the records use it for their fields.
pub(crate) trait IntoOwned {
    /// The type that owns its bytes.
    type Owned;

    /// Return a value that owns its bytes.
    fn to_static(self) -> Self::Owned;
}

impl IntoOwned for Bytes<'_> {
    type Owned = Bytes<'static>;

    fn to_static(self) -> Bytes<'static> {
        self.into_owned()
    }
}

impl<T: IntoOwned> IntoOwned for Option<T> {
    type Owned = Option<T::Owned>;

    fn to_static(self) -> Self::Owned {
        self.map(IntoOwned::to_static)
    }
}

impl<T: IntoOwned> IntoOwned for Vec<T> {
    type Owned = Vec<T::Owned>;

    fn to_static(self) -> Self::Owned {
        self.into_iter().map(IntoOwned::to_static).collect()
    }
}

impl<T: IntoOwned> IntoOwned for Box<T> {
    type Owned = Box<T::Owned>;

    fn to_static(self) -> Self::Owned {
        Box::new((*self).to_static())
    }
}

/// Implement `into_owned` and [`IntoOwned`] for a record type with a lifetime.
///
/// - `struct Name { owned fields; other fields }`: the owned fields hold
///   bytes or records with a lifetime. The other fields move without change.
/// - `enum Name { owned variants; other variants; unit variants }`: each owned
///   or other variant has one field.
macro_rules! into_owned {
    (struct $name:ident { $($own:ident),* $(,)? ; $($keep:ident),* $(,)? }) => {
        impl $name<'_> {
            /// Return a record that owns its bytes. Copy borrowed bytes.
            #[must_use]
            pub fn into_owned(self) -> $name<'static> {
                $name {
                    $($own: $crate::bytes::IntoOwned::to_static(self.$own),)*
                    $($keep: self.$keep,)*
                }
            }
        }

        impl $crate::bytes::IntoOwned for $name<'_> {
            type Owned = $name<'static>;

            fn to_static(self) -> $name<'static> {
                self.into_owned()
            }
        }
    };
    (
        enum $name:ident {
            $($own:ident),* $(,)? ;
            $($keep:ident),* $(,)? ;
            $($unit:ident),* $(,)?
        }
    ) => {
        impl $name<'_> {
            /// Return a record that owns its bytes. Copy borrowed bytes.
            #[must_use]
            pub fn into_owned(self) -> $name<'static> {
                match self {
                    $(Self::$own(value) => $name::$own(
                        $crate::bytes::IntoOwned::to_static(value)
                    ),)*
                    $(Self::$keep(value) => $name::$keep(value),)*
                    $(Self::$unit => $name::$unit,)*
                }
            }
        }

        impl $crate::bytes::IntoOwned for $name<'_> {
            type Owned = $name<'static>;

            fn to_static(self) -> $name<'static> {
                self.into_owned()
            }
        }
    };
}

pub(crate) use into_owned;
