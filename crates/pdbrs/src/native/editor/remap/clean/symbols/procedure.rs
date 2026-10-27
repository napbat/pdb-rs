//! Procedures and separated code.
//!
//! The part of a procedure with its start keeps the procedure record, with the length of
//! that part. Each other part becomes an `S_SEPCODE` scope without children, with the new
//! procedure start as its parent address. MSVC writes `S_SEPCODE` at the top level, after
//! the end of its procedure, with flags 0 (fixture `msvc-x64-pgo`). The new scopes go
//! after the `S_SEPCODE` scopes that follow the procedure with the same parent address.

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::classify::{self, Code};
use super::{Frame, Moved, Pending, Split, Walker, follows, push};
use crate::Result;
use crate::native::RemapItem;
use crate::native::codeview::symbol::{
    S_END, S_SEPCODE, SeparatedCode, SeparatedCodeFlags, SymbolData,
};
use crate::native::editor::record::SymbolRecord;
use crate::native::editor::remap::clean::record_error;
use crate::native::editor::tree::{ModuleSymbol, Scope};
use crate::native::output::no_memory;
use crate::native::remap::clean::Point;
use crate::{Error, ErrorKind};

impl Walker<'_, '_> {
    /// Rewrite a procedure scope with the code range `code` and append what stays to `out`.
    /// `rest` holds the next nodes of the list in reverse order. Return the new `S_SEPCODE`
    /// scopes of a procedure that splits.
    pub(super) fn procedure(
        &mut self,
        mut scope: Box<Scope>,
        code: Code,
        out: &mut Vec<ModuleSymbol>,
        rest: &[ModuleSymbol],
    ) -> Result<Option<Pending>> {
        let map = self.context.map;
        let Some(parts) = map.section_parts(code.address, code.length)? else {
            self.list(&mut scope.children, Frame::Keep)?;
            push(out, ModuleSymbol::Scope(scope))?;
            return Ok(None);
        };
        let rva = self.rva(code.address);
        let Some(first) = parts.parts.first().copied() else {
            self.remove(&scope.opener, rva)?;
            self.dropped.try_reserve(1).map_err(|_| no_memory())?;
            self.dropped.push(code.address);
            self.forget_scope(&scope);
            return Ok(None);
        };
        if first.offset != 0 {
            return Err(record_error(
                RemapItem::Symbol(scope.opener.kind()),
                rva,
                self.location(&scope.opener),
                "the start of a procedure has no remap range, but another part of it has one",
            ));
        }
        let start = self.new_address(first.rva)?;
        if parts.is_whole() {
            if start != code.address {
                classify::set_code(
                    scope.opener.data_mut(),
                    Code {
                        address: start,
                        ..code
                    },
                );
            }
            self.list(&mut scope.children, Frame::Keep)?;
            push(out, ModuleSymbol::Scope(scope))?;
            return Ok(None);
        }
        if parts.removed {
            self.remove(&scope.opener, rva)?;
        }
        let followers = rest
            .iter()
            .rev()
            .take_while(|node| follows(node, code.address))
            .count();
        let seen = self
            .separated
            .iter()
            .filter(|parent| **parent == code.address)
            .count();
        let first_chunk = u32::try_from(1 + seen + followers).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("a procedure has too many separated code chunks")
        })?;
        let split = Split {
            start: rva,
            length: code.length,
            parts,
            first_chunk,
        };
        self.split.try_reserve(1).map_err(|_| no_memory())?;
        self.split.push((code.address, rva));
        self.list(&mut scope.children, Frame::Split(&split))?;
        classify::set_code(
            scope.opener.data_mut(),
            Code {
                address: start,
                length: first.length,
                debug_start: code.debug_start.min(first.length),
                debug_end: code.debug_end.min(first.length),
            },
        );
        push(out, ModuleSymbol::Scope(scope))?;
        let others = &split.parts.parts[1..];
        let mut scopes = Vec::new();
        scopes.try_reserve(others.len()).map_err(|_| no_memory())?;
        for part in others {
            let opener = SymbolRecord::new(
                S_SEPCODE,
                SymbolData::SeparatedCode(SeparatedCode {
                    code_length: part.length,
                    flags: SeparatedCodeFlags(0),
                    address: self.new_address(part.rva)?,
                    parent_address: start,
                }),
            );
            let closer = SymbolRecord::new(S_END, SymbolData::ScopeEnd);
            scopes.push(ModuleSymbol::Scope(Box::new(Scope::new(opener, closer))));
        }
        self.context
            .added(RemapItem::Symbol(S_SEPCODE), scopes.len());
        Ok(Some((code.address, scopes)))
    }

    /// Rewrite an `S_SEPCODE` scope with the fields `value` and append it to `out` if it
    /// stays. The scope moves with its range; a split fails. The parent address maps as a
    /// point.
    pub(super) fn separated_code(
        &mut self,
        mut scope: Box<Scope>,
        value: SeparatedCode,
        out: &mut Vec<ModuleSymbol>,
    ) -> Result<()> {
        let parent = value.parent_address;
        self.separated.try_reserve(1).map_err(|_| no_memory())?;
        self.separated.push(parent);
        if self.dropped.contains(&parent) {
            // The procedure of the chunk went away, so the chunk goes too.
            let rva = self.rva(value.address);
            self.remove(&scope.opener, rva)?;
            self.forget_scope(&scope);
            return Ok(());
        }
        let mut new = value;
        match self.extent(&scope.opener, value.address, value.code_length)? {
            Moved::Keep => {}
            Moved::Move(address) => new.address = address,
            Moved::Remove => {
                self.forget_scope(&scope);
                return Ok(());
            }
        }
        match self.context.map.point(parent) {
            Point::Absent => {}
            Point::Removed(rva) => {
                self.remove(&scope.opener, rva)?;
                self.forget_scope(&scope);
                return Ok(());
            }
            Point::Mapped(address) => new.parent_address = address,
        }
        if new != value
            && let SymbolData::SeparatedCode(fields) = scope.opener.data_mut()
        {
            *fields = new;
        }
        let frame = self
            .split
            .iter()
            .find(|(start, _)| *start == parent)
            .map_or(Frame::Keep, |&(_, rva)| Frame::Fail(rva));
        self.list(&mut scope.children, frame)?;
        push(out, ModuleSymbol::Scope(scope))
    }
}
