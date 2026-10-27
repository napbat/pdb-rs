//! The clean rewrite of the module symbols (#43, decisions 4 and 5).
//!
//! The rewrite walks the scope tree of a module in stream order. [`classify::shape`] gives
//! the address form of each record:
//!
//! - A point maps to its new address.
//! - A record with a code range and no split form (for example `S_BLOCK32`) moves with its
//!   range. A split fails with [`ErrorKind::UnsupportedEdit`].
//! - A procedure keeps the part with its start. Each other part becomes a new `S_SEPCODE`
//!   scope after the procedure ([`procedure`]).
//! - A def-range gets one record for each part ([`defrange`]).
//! - An inline site of a procedure that splits gets new code offsets ([`inline`]).
//!
//! A record that holds removed code makes the rewrite fail with [`RemovedCode::Fail`]. With
//! [`RemovedCode::Drop`], the rewrite removes the record (with its scope) and adds it to
//! [`Removed`].

mod classify;
mod defrange;
mod inline;
mod procedure;

use alloc::boxed::Box;
use alloc::vec::Vec;

use self::classify::{Shape, shape};
use super::{Context, Removed, no_split_form, record_error};
use crate::native::address::SectionOffset;
use crate::native::codeview::symbol::{SymbolData, Trampoline};
use crate::native::editor::module::{ModuleContents, Symbols, symbol_base};
use crate::native::editor::record::SymbolRecord;
use crate::native::editor::tree::{ModuleSymbol, Scope};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::remap::clean::{Parts, Point};
use crate::native::{RemapItem, RemovedCode};
use crate::{Error, ErrorKind, Result};

/// Rewrite the addresses of the symbols of `module`.
///
/// # Errors
///
/// - [`ErrorKind::UnsupportedEdit`] for opaque symbols, a record of a kind without a
///   decoder, a split without a split form, or removed code with [`RemovedCode::Fail`].
/// - [`ErrorKind::LimitExceeded`] if an allocation fails.
pub(super) fn rewrite(
    module: &mut ModuleContents,
    context: &mut Context<'_>,
    removed: &mut Removed,
) -> Result<()> {
    let index = module.index().0;
    let source = module
        .source()
        .and_then(|source| source.stream.as_ref())
        .map(|stream| (stream.stream(), symbol_base(stream)));
    match module.symbol_region_mut() {
        Symbols::Absent => Ok(()),
        Symbols::Opaque => {
            let error = Error::new(ErrorKind::UnsupportedEdit, None).with_context(
                "the symbols of a module do not form a scope tree, so a clean remap cannot \
                 rewrite their addresses",
            );
            Err(match source {
                Some((stream, _)) => error.with_stream(stream, 0),
                None => error,
            })
        }
        Symbols::Tree(nodes) => Walker {
            context,
            removed,
            module: index,
            source,
            separated: Vec::new(),
            dropped: Vec::new(),
            split: Vec::new(),
        }
        .list(nodes, Frame::Keep),
    }
}

/// The parts of a procedure that the map splits.
struct Split {
    /// The PDB RVA of the procedure start.
    start: u32,
    /// The old length of the procedure code.
    length: u32,
    /// The parts of the procedure code. The first part starts at offset 0.
    parts: Parts,
    /// The code chunk of the new `S_SEPCODE` record of the second part.
    first_chunk: u32,
}

/// The procedure around a record, for the code offsets of inline sites.
#[derive(Clone, Copy)]
enum Frame<'f> {
    /// The procedure does not split, or the record is outside a procedure: the inline
    /// sites keep their annotations.
    Keep,
    /// The record is in an `S_SEPCODE` scope of a procedure that splits. An inline site
    /// fails. The value is the PDB RVA of the procedure.
    Fail(u32),
    /// The procedure splits.
    Split(&'f Split),
}

/// The new place of a record with a code range.
enum Moved {
    /// The range is not an address, or it keeps its start.
    Keep,
    /// The new start.
    Move(SectionOffset),
    /// The range has no remap range: the caller removes the record.
    Remove,
}

/// New `S_SEPCODE` scopes and the old start of their procedure. They go after the
/// `S_SEPCODE` scopes that follow the procedure with the same parent address.
type Pending = (SectionOffset, Vec<ModuleSymbol>);

/// The state of the rewrite of one module.
struct Walker<'w, 'm> {
    /// The rewrite context.
    context: &'w mut Context<'m>,
    /// The removed records of all modules.
    removed: &'w mut Removed,
    /// The module index.
    module: u32,
    /// The source module stream and the module stream offset of its first symbol record.
    source: Option<(StreamId, u32)>,
    /// The old parent address of each `S_SEPCODE` record so far, in stream order.
    separated: Vec<SectionOffset>,
    /// The old start of each removed procedure.
    dropped: Vec<SectionOffset>,
    /// The old start and the PDB RVA of each procedure that splits.
    split: Vec<(SectionOffset, u32)>,
}

impl Walker<'_, '_> {
    /// Return the source module stream offset of `record`, if it comes from the source.
    fn offset(&self, record: &SymbolRecord) -> Option<u32> {
        let (_, base) = self.source?;
        base.checked_add(record.origin()?.record().offset)
    }

    /// Return the stream location of `record` for an error.
    fn location(&self, record: &SymbolRecord) -> Option<(StreamId, u64)> {
        let (stream, _) = self.source?;
        Some((stream, u64::from(self.offset(record)?)))
    }

    /// Return the PDB RVA of an old address, or 0 if the value is not an address.
    fn rva(&self, address: SectionOffset) -> u32 {
        self.context.map.old_rva(address).unwrap_or(0)
    }

    /// Return the address in the new sections of the new RVA `rva`.
    fn new_address(&self, rva: u32) -> Result<SectionOffset> {
        self.context.map.section_offset(rva).ok_or_else(|| {
            Error::new(ErrorKind::InvalidFormat, None)
                .with_context("a remap range lies outside the new section headers")
        })
    }

    /// Handle removed code in `record` at the PDB RVA `rva`: fail, or count the removal.
    fn remove(&mut self, record: &SymbolRecord, rva: u32) -> Result<()> {
        let location = self.location(record);
        self.context
            .remove(RemapItem::Symbol(record.kind()), rva, location)
    }

    /// Return the error for a split of `record` without a split form.
    fn no_split(&self, record: &SymbolRecord, rva: u32) -> Error {
        no_split_form(RemapItem::Symbol(record.kind()), rva, self.location(record))
    }

    /// Add `record` to the removed records.
    fn forget(&mut self, record: &SymbolRecord) {
        if let Some(id) = record.id() {
            self.removed.records.insert((self.module, id));
        }
        if let Some(offset) = self.offset(record) {
            self.removed.offsets.insert((self.module, offset));
        }
    }

    /// Add each record of `nodes` to the removed records.
    fn forget_nodes(&mut self, nodes: &[ModuleSymbol]) {
        for node in nodes {
            match node {
                ModuleSymbol::Record(record) => self.forget(record),
                ModuleSymbol::Scope(scope) => self.forget_scope(scope),
            }
        }
    }

    /// Add the records of `scope` to the removed records.
    fn forget_scope(&mut self, scope: &Scope) {
        self.forget(&scope.opener);
        self.forget_nodes(&scope.children);
        self.forget(&scope.closer);
    }

    /// Rewrite the nodes of one scope (or of the top level) in stream order.
    fn list(&mut self, nodes: &mut Vec<ModuleSymbol>, frame: Frame<'_>) -> Result<()> {
        let mut rest = core::mem::take(nodes);
        rest.reverse();
        nodes.try_reserve(rest.len()).map_err(|_| no_memory())?;
        let mut pending: Option<Pending> = None;
        while let Some(node) = rest.pop() {
            if let Some((parent, _)) = &pending
                && !follows(&node, *parent)
                && let Some((_, scopes)) = pending.take()
            {
                append(nodes, scopes)?;
            }
            match node {
                ModuleSymbol::Record(record) => self.record(record, nodes)?,
                ModuleSymbol::Scope(scope) => {
                    if let Some(next) = self.scope(scope, frame, nodes, &rest)? {
                        pending = Some(next);
                    }
                }
            }
        }
        if let Some((_, scopes)) = pending {
            append(nodes, scopes)?;
        }
        Ok(())
    }

    /// Rewrite a record that does not open a scope and append what stays to `out`.
    fn record(&mut self, mut record: SymbolRecord, out: &mut Vec<ModuleSymbol>) -> Result<()> {
        let keep = match shape(record.data()) {
            Shape::Opaque => {
                return Err(record_error(
                    RemapItem::Symbol(record.kind()),
                    0,
                    self.location(&record),
                    "a symbol record of a kind without a decoder can hold an address",
                ));
            }
            Shape::Points => self.points(&mut record)?,
            Shape::Extent { start, length } => match self.extent(&record, start, length)? {
                Moved::Keep => true,
                Moved::Move(start) => {
                    classify::set_extent_start(record.data_mut(), start);
                    true
                }
                Moved::Remove => false,
            },
            Shape::Trampoline(old) => self.trampoline(&mut record, old)?,
            Shape::Section(rva) => self.section(&mut record, rva)?,
            Shape::DefRange(range) => return defrange::rewrite(self, record, range, out),
            // These kinds open a scope, so the tree has them as scopes.
            Shape::Procedure(_) | Shape::SeparatedCode(_) | Shape::InlineSite | Shape::None => true,
        };
        if keep {
            push(out, ModuleSymbol::Record(record))
        } else {
            self.forget(&record);
            Ok(())
        }
    }

    /// Rewrite a scope and append what stays to `out`. `rest` holds the next nodes of the
    /// list in reverse order. Return the new `S_SEPCODE` scopes of a procedure that splits.
    fn scope(
        &mut self,
        mut scope: Box<Scope>,
        frame: Frame<'_>,
        out: &mut Vec<ModuleSymbol>,
        rest: &[ModuleSymbol],
    ) -> Result<Option<Pending>> {
        match shape(scope.opener.data()) {
            Shape::Procedure(code) => return self.procedure(scope, code, out, rest),
            Shape::SeparatedCode(value) => {
                self.separated_code(scope, value, out)?;
                return Ok(None);
            }
            Shape::InlineSite => {
                self.inline_site(scope, frame, out)?;
                return Ok(None);
            }
            Shape::Extent { start, length } => match self.extent(&scope.opener, start, length)? {
                Moved::Keep => {}
                Moved::Move(start) => classify::set_extent_start(scope.opener.data_mut(), start),
                Moved::Remove => {
                    self.forget_scope(&scope);
                    return Ok(None);
                }
            },
            Shape::Opaque => {
                return Err(record_error(
                    RemapItem::Symbol(scope.opener.kind()),
                    0,
                    self.location(&scope.opener),
                    "a symbol record of a kind without a decoder can hold an address",
                ));
            }
            // Only the kinds above open a scope.
            Shape::None
            | Shape::Points
            | Shape::Trampoline(_)
            | Shape::Section(_)
            | Shape::DefRange(_) => {}
        }
        self.list(&mut scope.children, frame)?;
        push(out, ModuleSymbol::Scope(scope))?;
        Ok(None)
    }

    /// Map the points of `record`. Return `false` if the record goes away.
    fn points(&mut self, record: &mut SymbolRecord) -> Result<bool> {
        let old = classify::points(record.data());
        let mut new = old;
        let mut removed = None;
        for value in new.iter_mut().flatten() {
            match self.context.map.point(*value) {
                Point::Absent => {}
                Point::Removed(rva) => {
                    removed.get_or_insert(rva);
                }
                Point::Mapped(address) => *value = address,
            }
        }
        if let Some(rva) = removed {
            self.remove(record, rva)?;
            return Ok(false);
        }
        if new != old {
            classify::set_points(record.data_mut(), new);
        }
        Ok(true)
    }

    /// Map the code range of a record without a split form.
    fn extent(
        &mut self,
        record: &SymbolRecord,
        start: SectionOffset,
        length: u32,
    ) -> Result<Moved> {
        let Some(parts) = self.context.map.section_parts(start, length)? else {
            return Ok(Moved::Keep);
        };
        let rva = self.rva(start);
        let Some(first) = parts.parts.first() else {
            self.remove(record, rva)?;
            return Ok(Moved::Remove);
        };
        if parts.parts.len() > 1 {
            return Err(self.no_split(record, rva));
        }
        if parts.removed {
            // Only a part of the range has a remap range, and the kind cannot shrink.
            if matches!(self.context.policy, RemovedCode::Fail) {
                self.remove(record, rva)?;
            }
            return Err(self.no_split(record, rva));
        }
        let new = self.new_address(first.rva)?;
        Ok(if new == start {
            Moved::Keep
        } else {
            Moved::Move(new)
        })
    }

    /// Map the thunk range and the target of an `S_TRAMPOLINE` record. Return `false` if the
    /// record goes away.
    fn trampoline(&mut self, record: &mut SymbolRecord, old: Trampoline) -> Result<bool> {
        let mut new = old;
        match self.extent(record, old.thunk, u32::from(old.thunk_size))? {
            Moved::Keep => {}
            Moved::Move(start) => new.thunk = start,
            Moved::Remove => return Ok(false),
        }
        match self.context.map.point(old.target) {
            Point::Absent => {}
            Point::Removed(rva) => {
                self.remove(record, rva)?;
                return Ok(false);
            }
            Point::Mapped(address) => new.target = address,
        }
        if new != old
            && let SymbolData::Trampoline(value) = record.data_mut()
        {
            *value = new;
        }
        Ok(true)
    }

    /// Map the RVA of an `S_SECTION` record and set the new section number. Return `false`
    /// if the record goes away.
    fn section(&mut self, record: &mut SymbolRecord, rva: u32) -> Result<bool> {
        // An RVA of 0 is not an address.
        if rva == 0 {
            return Ok(true);
        }
        let new = match self.context.map.rva_point(rva) {
            Point::Absent => return Ok(true),
            Point::Removed(rva) => {
                self.remove(record, rva)?;
                return Ok(false);
            }
            Point::Mapped(new) => new,
        };
        let number = self.new_address(new)?.section;
        if let SymbolData::Section(old) = record.data()
            && (old.rva != new || old.number != number)
            && let SymbolData::Section(value) = record.data_mut()
        {
            value.rva = new;
            value.number = number;
        }
        Ok(true)
    }

    /// Rewrite an inline site scope and append it to `out` if it stays.
    fn inline_site(
        &mut self,
        mut scope: Box<Scope>,
        frame: Frame<'_>,
        out: &mut Vec<ModuleSymbol>,
    ) -> Result<()> {
        match frame {
            Frame::Keep => {}
            Frame::Fail(rva) => return Err(self.no_split(&scope.opener, rva)),
            Frame::Split(split) => {
                let SymbolData::InlineSite(site) = scope.opener.data() else {
                    return push(out, ModuleSymbol::Scope(scope));
                };
                let location = self.location(&scope.opener);
                let item = RemapItem::Symbol(scope.opener.kind());
                match inline::rewrite(self.context.map, split, &site.annotations, item, location)? {
                    inline::Site::Keep => {}
                    inline::Site::Removed(rva) => {
                        self.remove(&scope.opener, rva)?;
                        self.forget_scope(&scope);
                        return Ok(());
                    }
                    inline::Site::Annotations(annotations) => {
                        if let SymbolData::InlineSite(site) = scope.opener.data_mut() {
                            site.annotations = annotations;
                        }
                    }
                }
            }
        }
        self.list(&mut scope.children, frame)?;
        push(out, ModuleSymbol::Scope(scope))
    }
}

/// Return `true` if `node` is an `S_SEPCODE` scope with the old parent address `parent`.
fn follows(node: &ModuleSymbol, parent: SectionOffset) -> bool {
    matches!(
        node,
        ModuleSymbol::Scope(scope)
            if matches!(scope.opener.data(), SymbolData::SeparatedCode(value) if value.parent_address == parent)
    )
}

/// Append `node` to `list` with a checked allocation.
fn push(list: &mut Vec<ModuleSymbol>, node: ModuleSymbol) -> Result<()> {
    list.try_reserve(1).map_err(|_| no_memory())?;
    list.push(node);
    Ok(())
}

/// Append `nodes` to `list` with a checked allocation.
fn append(list: &mut Vec<ModuleSymbol>, nodes: Vec<ModuleSymbol>) -> Result<()> {
    list.try_reserve(nodes.len()).map_err(|_| no_memory())?;
    list.extend(nodes);
    Ok(())
}
