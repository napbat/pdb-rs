//! Encoding of typed symbol records.
//!
//! [`SymbolData::encode`] writes one complete record: the length field, the
//! kind, the fields of the kind and the zero padding to a multiple of four
//! bytes. The field order is the order of the decoders. A value that the
//! decoder gives for a record encodes to the bytes of that record.
//!
//! A record that starts a scope also has the parent and end links. The
//! fields do not hold them, and the encoder does not invent them:
//!
//! - [`SymbolData::encode`] rejects a record that starts a scope.
//! - [`SymbolData::encode_scope`] writes the links that the caller gives.
//! - [`Symbol::encode`] writes the links of the source record.
//! - [`ModuleSymbolWriter`](super::ModuleSymbolWriter) computes the links.

use alloc::vec::Vec;

use super::super::writer::Padding;
use super::super::{Fields, SymbolKind};
use super::procedure::write_id_list;
use super::{
    ProcedureType, S_ANNOTATION, S_ANNOTATIONREF, S_ARMSWITCHTABLE, S_ASSOCIATION, S_ATTR_FRAMEREL,
    S_ATTR_MANYREG, S_ATTR_REGISTER, S_ATTR_REGREL, S_BLOCK32, S_BPREL32, S_BPREL32_ENCTMP,
    S_BPREL32_INDIR, S_BPREL32_INDIR_ENCTMP, S_BUILDINFO, S_CALLEES, S_CALLERS, S_CALLSITEINFO,
    S_COBOLUDT, S_COFFGROUP, S_COMPILE2, S_COMPILE3, S_CONSTANT, S_DATAREF, S_DEFRANGE,
    S_DEFRANGE_CONSTVAL_ON_ENTRY, S_DEFRANGE_DPC_PTR_TAG, S_DEFRANGE_FRAMEPOINTER_REL,
    S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE, S_DEFRANGE_GLOBALSYM_ON_ENTRY, S_DEFRANGE_HLSL,
    S_DEFRANGE_REGISTER, S_DEFRANGE_REGISTER_REL, S_DEFRANGE_REGISTER_REL_INDIR,
    S_DEFRANGE_SUBFIELD, S_DEFRANGE_SUBFIELD_REGISTER, S_DISCARDED, S_DPC_SYM_TAG_MAP, S_END,
    S_ENVBLOCK, S_EXPORT, S_FASTLINK, S_FILESTATIC, S_FRAMECOOKIE, S_FRAMEPROC, S_FRAMEREG,
    S_GDATA_HLSL, S_GDATA_HLSL32, S_GDATA_HLSL32_EX, S_GDATA32, S_GMANDATA, S_GMANPROC, S_GPROC32,
    S_GPROC32_ID, S_GPROC32EX, S_GPROC32EX_ID, S_GPROCIA64, S_GPROCIA64_ID, S_GPROCMIPS,
    S_GPROCMIPS_ID, S_GTHREAD32, S_HEAPALLOCSITE, S_HOTPATCHFUNC, S_HYBRIDRANGE, S_INLINEES,
    S_INLINESITE, S_INLINESITE_END, S_INLINESITE2, S_LABEL32, S_LDATA_HLSL, S_LDATA_HLSL32,
    S_LDATA_HLSL32_EX, S_LDATA32, S_LMANDATA, S_LMANPROC, S_LOCAL, S_LOCAL_DPC_GROUPSHARED,
    S_LOCALSLOT, S_LPROC32, S_LPROC32_DPC, S_LPROC32_DPC_ID, S_LPROC32_ID, S_LPROC32EX,
    S_LPROC32EX_ID, S_LPROCIA64, S_LPROCIA64_ID, S_LPROCMIPS, S_LPROCMIPS_ID, S_LPROCREF,
    S_LTHREAD32, S_MANCONSTANT, S_MANFRAMEREL, S_MANMANYREG, S_MANMANYREG2, S_MANREGISTER,
    S_MANREGREL, S_MANSLOT, S_MANTYPREF, S_MANYREG, S_MANYREG2, S_MOD_TYPEREF, S_OBJNAME, S_OEM,
    S_PARAMSLOT, S_PDBMAP, S_POGODATA, S_PROC_ID_END, S_PROCREF, S_PUB32, S_REF_MINIPDB,
    S_REGISTER, S_REGREL32, S_REGREL32_ENCTMP, S_REGREL32_INDIR, S_REGREL32_INDIR_ENCTMP,
    S_SECTION, S_SEPCODE, S_SKIP, S_SOURCELINK, S_STATICLOCAL, S_THUNK32, S_TOKENREF, S_TRAMPOLINE,
    S_UDT, S_UNAMESPACE, S_VFTABLE32, S_WITH32, SHORT_PAYLOAD, ScopeLinks, Symbol, SymbolData,
    decode_data,
};
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

impl SymbolData<'_> {
    /// Append the record of `kind` with these fields to `out`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the variant does not belong to
    ///   `kind`, for example [`SymbolData::Data`] with [`S_PUB32`], or if a
    ///   field breaks a rule of the decoder, for example a name with a NUL
    ///   byte. [`SymbolData::Unsupported`] is valid only for a kind that
    ///   this crate does not decode.
    /// - [`ErrorKind::InvalidFormat`] if the record starts a scope. Use
    ///   [`SymbolData::encode_scope`] or
    ///   [`ModuleSymbolWriter`](super::ModuleSymbolWriter) for it.
    /// - [`ErrorKind::InvalidReference`] for a reference with a
    ///   [`ModuleSymbolTarget::Handle`](super::ModuleSymbolTarget::Handle).
    ///   Only a write plan resolves the handle.
    /// - [`ErrorKind::LimitExceeded`] if the record is longer than its
    ///   `u16` length field allows, a count does not fit its field, or the
    ///   output cannot grow.
    ///
    /// After an error, `out` can hold a part of the record.
    pub fn encode(&self, kind: SymbolKind, out: &mut Vec<u8>) -> Result<()> {
        if self.starts_scope() {
            return Err(Error::new(ErrorKind::InvalidFormat, None).with_context(
                "symbol record starts a scope; encode it with its links (encode_scope) \
                 or with a ModuleSymbolWriter",
            ));
        }
        self.encode_with(kind, ScopeLinks::default(), Padding::Zero, out)
    }

    /// Append the record of `kind` with these fields and the scope links
    /// `links` to `out`.
    ///
    /// Use this function for a record that starts a scope, for example
    /// [`S_GPROC32`]. The links are offsets in the module stream:
    /// [`ModuleSymbolWriter`](super::ModuleSymbolWriter) computes them for
    /// a whole module.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the record does not start a scope.
    /// - The other errors of [`SymbolData::encode`].
    ///
    /// After an error, `out` can hold a part of the record.
    pub fn encode_scope(
        &self,
        kind: SymbolKind,
        links: ScopeLinks,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        if !self.starts_scope() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("symbol record with scope links does not start a scope"));
        }
        self.encode_with(kind, links, Padding::Zero, out)
    }

    /// Append the record of `kind` with these fields and `padding` to `out`.
    ///
    /// A record that starts a scope gets `links`. The other records ignore
    /// `links`.
    pub(super) fn encode_with(
        &self,
        kind: SymbolKind,
        links: ScopeLinks,
        padding: Padding,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        let mut writer = Output::new(out);
        let start = writer.begin_record(kind)?;
        let written = self.write_variable(kind, &mut writer)?
            || self.write_fields(kind, links, &mut writer)?;
        if !written {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("symbol data does not belong to the record kind"));
        }
        writer.finish_record(start, padding)
    }

    /// Write the fields of a kind that is not a variable, data or def-range
    /// kind. Return `false` if the variant does not belong to `kind`.
    fn write_fields(
        &self,
        kind: SymbolKind,
        links: ScopeLinks,
        out: &mut Output<'_>,
    ) -> Result<bool> {
        let id = matches!(
            kind,
            S_GPROC32_ID
                | S_LPROC32_ID
                | S_LPROC32_DPC_ID
                | S_GPROC32EX_ID
                | S_LPROC32EX_ID
                | S_GPROCMIPS_ID
                | S_LPROCMIPS_ID
                | S_GPROCIA64_ID
                | S_LPROCIA64_ID
        );
        let extended = matches!(
            kind,
            S_GPROC32EX | S_LPROC32EX | S_GPROC32EX_ID | S_LPROC32EX_ID
        );
        let signature_matches =
            |signature: ProcedureType| id == matches!(signature, ProcedureType::Id(_));
        match (kind, self) {
            (S_OBJNAME, Self::ObjectName(value)) => value.write(out)?,
            (S_COMPILE2, Self::Compile2(value)) => value.write(out)?,
            (S_COMPILE3, Self::Compile3(value)) => value.write(out)?,
            (S_BUILDINFO, Self::BuildInfo(index)) | (S_SOURCELINK, Self::SourceLink(index)) => {
                out.id_index(*index)?;
            }
            (S_ENVBLOCK, Self::EnvironmentBlock(value)) => value.write(out)?,
            (S_UNAMESPACE, Self::UsingNamespace(name)) => out.c_string(name)?,
            (
                S_GPROC32 | S_LPROC32 | S_LPROC32_DPC | S_GPROC32_ID | S_LPROC32_ID
                | S_LPROC32_DPC_ID | S_GPROC32EX | S_LPROC32EX | S_GPROC32EX_ID | S_LPROC32EX_ID,
                Self::Procedure(value),
            ) if signature_matches(value.signature)
                && extended == value.extended_flags.is_some() =>
            {
                value.write(links, out)?;
            }
            (
                S_GPROCMIPS | S_LPROCMIPS | S_GPROCMIPS_ID | S_LPROCMIPS_ID,
                Self::MipsProcedure(value),
            ) if signature_matches(value.signature) => value.write(links, out)?,
            (
                S_GPROCIA64 | S_LPROCIA64 | S_GPROCIA64_ID | S_LPROCIA64_ID,
                Self::Ia64Procedure(value),
            ) if signature_matches(value.signature) => value.write(links, out)?,
            (S_FRAMEPROC, Self::FrameProcedure(value)) => value.write(out)?,
            (S_BLOCK32, Self::Block(value)) => value.write(links, out)?,
            (S_WITH32, Self::With(value)) => value.write(links, out)?,
            (S_SEPCODE, Self::SeparatedCode(value)) => value.write(links, out)?,
            (S_LABEL32, Self::Label(value)) => value.write(out)?,
            (S_THUNK32, Self::Thunk(value)) => value.write(links, out)?,
            (S_END | S_PROC_ID_END, Self::ScopeEnd) | (S_INLINESITE_END, Self::InlineSiteEnd) => {}
            (S_INLINESITE | S_INLINESITE2, Self::InlineSite(value))
                if (kind == S_INLINESITE2) == value.invocations.is_some() =>
            {
                value.write(links, out)?;
            }
            (S_INLINEES, Self::Inlinees(value)) => write_id_list(value, out)?,
            (S_CALLEES, Self::Callees(value)) | (S_CALLERS, Self::Callers(value)) => {
                value.write(out)?;
            }
            (S_CALLSITEINFO, Self::CallSiteInfo(value)) => value.write(out)?,
            (S_ARMSWITCHTABLE, Self::JumpTable(value)) => value.write(out)?,
            (S_POGODATA, Self::ProfileData(value)) => value.write(out)?,
            (S_HOTPATCHFUNC, Self::HotPatchFunction(value)) => value.write(out)?,
            (S_ASSOCIATION, Self::Association(value)) => value.write(out)?,
            (S_HYBRIDRANGE, Self::HybridRange(value)) => value.write(out)?,
            (S_ANNOTATION, Self::AnnotationStrings(value)) => value.write(out)?,
            (S_FRAMECOOKIE, Self::FrameCookie(value)) => value.write(out)?,
            (S_FRAMEREG, Self::FrameRegister(register)) => out.u16(*register)?,
            (S_HEAPALLOCSITE, Self::HeapAllocationSite(value)) => value.write(out)?,
            (S_TRAMPOLINE, Self::Trampoline(value)) => value.write(out)?,
            (S_FASTLINK | S_REF_MINIPDB, Self::FastLink(value))
                if (kind == S_REF_MINIPDB) == value.module.is_some() =>
            {
                value.write(out)?;
            }
            (S_SECTION, Self::Section(value)) => value.write(out)?,
            (S_COFFGROUP, Self::CoffGroup(value)) => value.write(out)?,
            (S_EXPORT, Self::Export(value)) => value.write(out)?,
            (S_DISCARDED, Self::Discarded(value)) => value.write(out)?,
            (S_MOD_TYPEREF, Self::ModuleTypeReference(value)) => value.write(out)?,
            (S_PDBMAP, Self::PdbMap(value)) => value.write(out)?,
            (S_VFTABLE32, Self::VirtualFunctionTable(value)) => value.write(out)?,
            (S_PUB32, Self::Public(value)) => value.write(out)?,
            (S_PROCREF | S_LPROCREF | S_TOKENREF, Self::ProcedureReference(value))
            | (S_DATAREF | S_ANNOTATIONREF, Self::SymbolReference(value)) => value.write(out)?,
            (S_GMANPROC | S_LMANPROC, Self::ManagedProcedure(value)) => value.write(links, out)?,
            (S_MANTYPREF, Self::ManagedTypeReference(index)) => out.type_index(*index)?,
            (S_OEM, Self::Oem(value)) => value.write(out)?,
            (S_SKIP, Self::Skip(bytes)) => out.bytes(bytes)?,
            (_, Self::Unsupported(bytes)) if !is_decoded_kind(kind) => out.bytes(bytes)?,
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Write the fields of a variable, data or def-range kind. Return
    /// `false` for other kinds and for a variant that does not belong to
    /// `kind`.
    fn write_variable(&self, kind: SymbolKind, out: &mut Output<'_>) -> Result<bool> {
        let attributed = !matches!(kind, S_MANYREG | S_MANYREG2);
        let wide = matches!(kind, S_MANYREG2 | S_MANMANYREG2 | S_ATTR_MANYREG);
        match (kind, self) {
            (S_LOCAL, Self::Local(value)) => value.write(out)?,
            (S_DEFRANGE | S_DEFRANGE_SUBFIELD, Self::DefRangeProgram(value))
                if (kind == S_DEFRANGE_SUBFIELD) == value.parent_offset.is_some() =>
            {
                value.write(out)?;
            }
            (S_DEFRANGE_FRAMEPOINTER_REL, Self::DefRangeFramePointerRelative(value)) => {
                value.write(out)?;
            }
            (
                S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE,
                Self::DefRangeFramePointerRelativeFullScope(offset),
            ) => out.i32(*offset)?,
            (S_DEFRANGE_REGISTER, Self::DefRangeRegister(value)) => value.write(out)?,
            (S_DEFRANGE_REGISTER_REL, Self::DefRangeRegisterRelative(value)) => value.write(out)?,
            (S_DEFRANGE_REGISTER_REL_INDIR, Self::DefRangeRegisterRelativeIndirect(value)) => {
                value.write(out)?;
            }
            (S_DEFRANGE_SUBFIELD_REGISTER, Self::DefRangeSubfieldRegister(value)) => {
                value.write(out)?;
            }
            (S_DEFRANGE_HLSL | S_DEFRANGE_DPC_PTR_TAG, Self::DefRangeHlsl(value)) => {
                value.write(out)?;
            }
            (S_DEFRANGE_CONSTVAL_ON_ENTRY, Self::DefRangeConstantOnEntry(value)) => {
                value.write(out)?;
            }
            (S_DEFRANGE_GLOBALSYM_ON_ENTRY, Self::DefRangeGlobalOnEntry(address)) => {
                address.write(out)?;
            }
            (S_REGREL32 | S_REGREL32_ENCTMP, Self::RegisterRelative(value)) => value.write(out)?,
            (S_REGREL32_INDIR | S_REGREL32_INDIR_ENCTMP, Self::RegisterRelativeIndirect(value)) => {
                value.write(out)?;
            }
            (S_BPREL32 | S_BPREL32_ENCTMP, Self::BasePointerRelative(value)) => {
                value.write(out)?;
            }
            (
                S_BPREL32_INDIR | S_BPREL32_INDIR_ENCTMP,
                Self::BasePointerRelativeIndirect(value),
            ) => value.write(out)?,
            (S_REGISTER, Self::Register(value)) => value.write(out)?,
            (
                S_MANYREG | S_MANYREG2 | S_MANMANYREG | S_MANMANYREG2 | S_ATTR_MANYREG,
                Self::ManyRegisters(value),
            ) if attributed == value.attributes.is_some() && wide == value.registers.is_wide() => {
                value.write(out)?;
            }
            (S_MANFRAMEREL | S_ATTR_FRAMEREL, Self::AttributedFrameRelative(value)) => {
                value.write(out)?;
            }
            (S_MANREGISTER | S_ATTR_REGISTER, Self::AttributedRegister(value)) => {
                value.write(out)?;
            }
            (S_MANREGREL | S_ATTR_REGREL, Self::AttributedRegisterRelative(value)) => {
                value.write(out)?;
            }
            (S_LOCALSLOT | S_PARAMSLOT, Self::Slot(value)) => value.write(out)?,
            (S_FILESTATIC, Self::FileStatic(value)) => value.write(out)?,
            (S_STATICLOCAL, Self::StaticLocal(value)) => value.write(out)?,
            (S_GTHREAD32 | S_LTHREAD32, Self::ThreadData(value))
            | (S_GDATA32 | S_LDATA32, Self::Data(value))
            | (S_GMANDATA | S_LMANDATA, Self::ManagedData(value)) => value.write(out)?,
            (S_UDT | S_COBOLUDT, Self::UserDefinedType(value)) => value.write(out)?,
            (S_GDATA_HLSL | S_LDATA_HLSL, Self::HlslData(value)) => value.write(out, false)?,
            (S_GDATA_HLSL32 | S_LDATA_HLSL32, Self::HlslData(value)) => value.write(out, true)?,
            (S_GDATA_HLSL32_EX | S_LDATA_HLSL32_EX, Self::HlslDataEx(value)) => value.write(out)?,
            (S_LOCAL_DPC_GROUPSHARED, Self::DpcGroupSharedLocal(value)) => value.write(out)?,
            (S_DPC_SYM_TAG_MAP, Self::DpcTagMap(value)) => value.write(out)?,
            (S_CONSTANT, Self::Constant(value)) => value.write(out)?,
            (S_MANSLOT, Self::ManagedSlot(value)) => value.write(out)?,
            (S_MANCONSTANT, Self::ManagedConstant(value)) => value.write(out)?,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

impl Symbol<'_> {
    /// Append the record of this symbol to `out`.
    ///
    /// For a symbol that [`Symbol::decode`] returned, the output is equal to
    /// the bytes of the original record. A record that starts a scope gets
    /// the links of the original record (see [`Symbol::links`]).
    ///
    /// # Errors
    ///
    /// The errors of [`SymbolData::encode`], but not the error for a record
    /// that starts a scope.
    ///
    /// The output has the length of the original record: the encoder writes the fields
    /// and then the zero bytes of the original record after them (see [`Symbol::trailing_zeros`]).
    /// A record with four or more zero bytes, or with a length that is not a multiple of 4,
    /// thus keeps its bytes.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let start = out.len();
        let links = self.links().unwrap_or_default();
        self.data
            .encode_with(self.kind(), links, Padding::None, out)?;
        // The record had a valid u16 length, so the original length fits.
        let original = self.payload.len() + 4;
        let extra = original.saturating_sub(out.len() - start);
        if extra == 0 {
            return Ok(());
        }
        out.try_reserve(extra).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("CodeView record output cannot be allocated")
        })?;
        out.resize(out.len() + extra, 0);
        let length = u16::try_from(original - 2).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("CodeView record is longer than its u16 length field allows")
        })?;
        out[start..start + 2].copy_from_slice(&length.to_le_bytes());
        Ok(())
    }
}

/// Return `true` if [`Symbol::decode`] gives typed fields for `kind`.
///
/// The decoder gives [`SymbolData::Unsupported`] only for kinds without a
/// decoder, and it does that without reading a field.
fn is_decoded_kind(kind: SymbolKind) -> bool {
    let mut fields = Fields::new(&[], ErrorKind::InvalidFormat, SHORT_PAYLOAD);
    !matches!(
        decode_data(kind, &mut fields),
        Ok(SymbolData::Unsupported(_))
    )
}
