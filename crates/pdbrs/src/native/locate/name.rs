//! Name queries through the global and public symbol indexes.

use alloc::vec::Vec;

use super::scope::optional_rva;
use super::{ProcedureInfo, ProcedureKey, PublicMatch, SymbolLocator, push};
use crate::native::address::Rva;
use crate::native::codeview::symbol::{
    ModuleSymbolTarget, ProcedureReference, Public, Symbol, SymbolData,
};
use crate::native::symbols::SymbolOffset;
use crate::{Bytes, Error, ErrorKind, Result};

/// A symbol with the name of a query.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum NameMatch<'a> {
    /// An `S_PROCREF`, `S_LPROCREF` or `S_TOKENREF` record of the global
    /// symbol index, with the module procedure that it refers to.
    Procedure {
        /// The offset of the reference in the symbol record stream.
        offset: SymbolOffset,
        /// The reference.
        reference: ProcedureReference<'a>,
        /// The procedure record in the module stream.
        procedure: ProcedureInfo<'a>,
    },
    /// Another record of the global symbol index, for example `S_GDATA32`,
    /// `S_CONSTANT` or `S_UDT`.
    Global {
        /// The offset of the record in the symbol record stream.
        offset: SymbolOffset,
        /// The decoded record.
        symbol: Symbol<'a>,
    },
    /// An `S_PUB32` record of the public symbol index.
    Public {
        /// The offset of the record in the symbol record stream.
        offset: SymbolOffset,
        /// The decoded record.
        public: Public<'a>,
        /// The RVA, or `None` if the address is not in an image section.
        rva: Option<Rva>,
    },
}

impl SymbolLocator {
    /// Find the global and public symbols with the exact name `name`.
    ///
    /// The function gets the candidates of the hash bucket of `name` from
    /// the global and the public symbol index ([`HashTable::lookup`]). It
    /// decodes each candidate and keeps the records with the same name
    /// bytes. The global matches come first, in bucket order, then the
    /// public matches.
    ///
    /// Global names of procedures are usually not decorated. Public names
    /// of C++ functions are usually decorated.
    ///
    /// [`HashTable::lookup`]: crate::native::symbols::HashTable::lookup
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if a hash record or a procedure
    ///   reference does not identify a record, or if the public index refers
    ///   to a record that is not `S_PUB32`.
    /// - The decoding errors of the records.
    pub fn find_name(&self, name: &[u8]) -> Result<Vec<NameMatch<'_>>> {
        let mut matches = Vec::new();
        if let Some(index) = &self.global_index {
            for candidate in index.hash_table().lookup(name) {
                let offset = candidate.symbol;
                let symbol = self.shared_symbol(offset)?;
                if symbol.name().map(Bytes::as_bytes) != Some(name) {
                    continue;
                }
                let found = match symbol.procedure_reference() {
                    Some(reference) => NameMatch::Procedure {
                        offset,
                        procedure: self.referenced_procedure(reference)?,
                        reference: reference.clone(),
                    },
                    None => NameMatch::Global { offset, symbol },
                };
                push(&mut matches, found, "name match list cannot be allocated")?;
            }
        }
        if let Some(index) = &self.public_index {
            let map = self.address_map()?;
            for candidate in index.hash_table().lookup(name) {
                let offset = candidate.symbol;
                let public = self.public(offset)?;
                if public.name != name {
                    continue;
                }
                let rva = optional_rva(&map, public.address)?;
                let found = NameMatch::Public {
                    offset,
                    public,
                    rva,
                };
                push(&mut matches, found, "name match list cannot be allocated")?;
            }
        }
        Ok(matches)
    }

    /// Return the public symbol at or before `rva` in the same section.
    ///
    /// Of public symbols at the same RVA, return the first one in the
    /// address map of the public symbol index.
    ///
    /// # Errors
    ///
    /// Returns the errors of the address translation and of decoding the
    /// public symbol.
    pub fn public_at(&self, rva: Rva) -> Result<Option<PublicMatch<'_>>> {
        let Some(span) = self.publics.preceding(rva.0) else {
            return Ok(None);
        };
        let map = self.address_map()?;
        let offset = span.value;
        let public = self.public(offset)?;
        let same_section = match map.section_offset(rva) {
            Ok(address) => address
                .mapped()
                .is_some_and(|address| address.section == public.address.section),
            Err(error) if error.kind() == ErrorKind::InvalidReference => false,
            Err(error) => return Err(error),
        };
        if !same_section {
            return Ok(None);
        }
        Ok(Some(PublicMatch {
            offset,
            public,
            rva: Rva(span.start),
            displacement: rva.0 - span.start,
        }))
    }

    /// Decode the record at an offset of the symbol record stream.
    fn shared_symbol(&self, offset: SymbolOffset) -> Result<Symbol<'_>> {
        Symbol::decode(offset.record(&self.symbol_records)?)
    }

    /// Decode the `S_PUB32` record at an offset of the symbol record stream.
    fn public(&self, offset: SymbolOffset) -> Result<Public<'_>> {
        match self.shared_symbol(offset)?.data() {
            SymbolData::Public(public) => Ok(public.clone()),
            _ => Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("public symbol index refers to a record that is not S_PUB32")),
        }
    }

    /// Decode the module procedure that a reference names.
    fn referenced_procedure(
        &self,
        reference: &ProcedureReference<'_>,
    ) -> Result<ProcedureInfo<'_>> {
        let ModuleSymbolTarget::Position {
            module: Some(module),
            offset,
        } = reference.target
        else {
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("procedure reference has module number 0"));
        };
        self.procedure_info(ProcedureKey { module, offset })
    }
}
