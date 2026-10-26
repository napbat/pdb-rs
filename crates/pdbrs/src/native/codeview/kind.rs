//! Typed `CodeView` record kinds.
//!
//! [`SymbolKind`] holds the kind of a symbol record (`SYM_ENUM_e`). [`LeafKind`] holds the
//! kind of a type record, an ID record, a member or a numeric leaf (`LEAF_ENUM_e`). Both keep
//! values that this crate does not know.
//!
//! `Debug` prints the `cvinfo.h` name of a known kind and the hexadecimal value of an unknown
//! kind. `LowerHex` and `UpperHex` format the raw value.

use core::fmt;

mod leaf_names;
mod symbol_names;

/// Find `raw` in a name table that is sorted by value.
const fn find(names: &[(u16, &'static str)], raw: u16) -> Option<&'static str> {
    let mut low = 0;
    let mut high = names.len();
    while low < high {
        let middle = low + (high - low) / 2;
        let (value, name) = names[middle];
        if value == raw {
            return Some(name);
        }
        if value < raw {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    None
}

/// Define a kind newtype with a `cvinfo.h` name table.
macro_rules! kind {
    ($(#[$meta:meta])* $name:ident, $names:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub u16);

        impl $name {
            /// Return the `cvinfo.h` name of the kind, or `None` for an unknown value.
            #[must_use]
            pub const fn name(self) -> Option<&'static str> {
                $names::name(self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self.name() {
                    Some(name) => formatter.write_str(name),
                    None => write!(formatter, concat!(stringify!($name), "({:#06x})"), self.0),
                }
            }
        }

        impl fmt::LowerHex for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::LowerHex::fmt(&self.0, formatter)
            }
        }

        impl fmt::UpperHex for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::UpperHex::fmt(&self.0, formatter)
            }
        }
    };
}

kind! {
    /// The kind of a symbol record (`SYM_ENUM_e` in `cvinfo.h`).
    SymbolKind, symbol_names
}

kind! {
    /// The kind of a type record, an ID record, a member or a numeric leaf (`LEAF_ENUM_e` in
    /// `cvinfo.h`).
    ///
    /// Two names share `0x8000`: `LF_NUMERIC` and `LF_CHAR`. `Debug` prints `LF_CHAR`.
    LeafKind, leaf_names
}

#[cfg(test)]
mod tests {
    use alloc::format;

    use super::{LeafKind, SymbolKind};

    #[test]
    fn debug_prints_the_cvinfo_name_or_the_hex_value() {
        assert_eq!(format!("{:?}", SymbolKind(0x1110)), "S_GPROC32");
        assert_eq!(format!("{:?}", SymbolKind(0x1168)), "S_INLINEES");
        assert_eq!(format!("{:?}", SymbolKind(0xfffe)), "SymbolKind(0xfffe)");
        assert_eq!(format!("{:?}", LeafKind(0x1504)), "LF_CLASS");
        assert_eq!(format!("{:?}", LeafKind(0x8000)), "LF_CHAR");
        assert_eq!(format!("{:?}", LeafKind(0x7ff0)), "LeafKind(0x7ff0)");
    }

    #[test]
    fn name_tables_are_sorted() {
        for names in [super::symbol_names::NAMES, super::leaf_names::NAMES] {
            assert!(names.windows(2).all(|pair| pair[0].0 < pair[1].0));
        }
    }

    #[test]
    fn hex_formats_the_raw_value() {
        assert_eq!(format!("{:#06x}", SymbolKind(0x110e)), "0x110e");
        assert_eq!(format!("{:X}", LeafKind(0x150c)), "150C");
    }
}
