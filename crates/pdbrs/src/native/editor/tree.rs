//! The scope tree of the symbols of a module (decision 3 of #42).
//!
//! These kinds open a scope: the procedures of all architectures and their
//! `_ID` kinds, the managed procedures, `S_BLOCK32`, `S_WITH32`, `S_SEPCODE`,
//! `S_THUNK32`, `S_INLINESITE` and `S_INLINESITE2`. `S_END` and
//! `S_PROC_ID_END` close a scope that is not an inline site.
//! `S_INLINESITE_END` closes an inline site.
//!
//! A scope node holds the opener record, the child nodes and the closer
//! record. The plan writes the tree in pre-order with
//! [`ModuleSymbolWriter`](crate::native::codeview::symbol::ModuleSymbolWriter),
//! which computes the parent and end links.

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::record::SymbolRecord;
use crate::native::codeview::SymbolKind;
use crate::native::codeview::index::ModuleSymbolOffset;
use crate::native::codeview::symbol::{
    S_END, S_INLINESITE, S_INLINESITE_END, S_INLINESITE2, S_PROC_ID_END, Symbol, SymbolData,
};
use crate::native::output::no_memory;
use crate::{Error, ErrorKind, Result};

/// A node of the scope tree of a module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModuleSymbol {
    /// A record that does not open a scope.
    Record(SymbolRecord),
    /// A scope: its opener, its children and its closer.
    Scope(Box<Scope>),
}

impl ModuleSymbol {
    /// Return the kind of the record, or the kind of the opener of a scope.
    #[must_use]
    pub const fn kind(&self) -> SymbolKind {
        self.record().kind()
    }

    /// Return the record, or the opener of a scope.
    #[must_use]
    pub const fn record(&self) -> &SymbolRecord {
        match self {
            Self::Record(record) => record,
            Self::Scope(scope) => &scope.opener,
        }
    }

    /// Return the record, or the opener of a scope, for a change.
    pub const fn record_mut(&mut self) -> &mut SymbolRecord {
        match self {
            Self::Record(record) => record,
            Self::Scope(scope) => &mut scope.opener,
        }
    }
}

/// A scope of the tree: a procedure, a block, a thunk or an inline site.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Scope {
    /// The record that opens the scope, for example `S_GPROC32`.
    pub opener: SymbolRecord,
    /// The nodes inside the scope, in stream order.
    pub children: Vec<ModuleSymbol>,
    /// The record that closes the scope, for example `S_END`.
    pub closer: SymbolRecord,
}

impl Scope {
    /// Make a scope with `opener`, no children and `closer`.
    #[must_use]
    pub const fn new(opener: SymbolRecord, closer: SymbolRecord) -> Self {
        Self {
            opener,
            children: Vec::new(),
            closer,
        }
    }
}

/// Return `true` if `kind` closes a scope.
pub(super) fn closes_scope(kind: SymbolKind) -> bool {
    matches!(kind, S_END | S_PROC_ID_END | S_INLINESITE_END)
}

/// Return `true` if `opener` and `closer` belong together.
fn matches(opener: SymbolKind, closer: SymbolKind) -> bool {
    let inline = matches!(opener, S_INLINESITE | S_INLINESITE2);
    if inline {
        closer == S_INLINESITE_END
    } else {
        matches!(closer, S_END | S_PROC_ID_END)
    }
}

/// Return `true` if `data` opens a scope.
pub(super) const fn opens_scope(data: &SymbolData<'_>) -> bool {
    data.starts_scope()
}

/// An open scope while the tree is built.
#[derive(Clone, Debug)]
pub(super) struct Open {
    /// The opener.
    opener: SymbolRecord,
    /// The children so far.
    children: Vec<ModuleSymbol>,
    /// The module stream offset of the opener in the source, or 0.
    offset: u32,
}

/// Push `node` to the innermost open scope or to the top level.
fn attach(top: &mut Vec<ModuleSymbol>, open: &mut [Open], node: ModuleSymbol) -> Result<()> {
    let list = match open.last_mut() {
        Some(scope) => &mut scope.children,
        None => top,
    };
    list.try_reserve(1).map_err(|_| no_memory())?;
    list.push(node);
    Ok(())
}

/// The result of a push to a tree under construction.
pub(super) enum Pushed {
    /// The record went into the tree.
    Done,
    /// The record is a closer without an open scope, or of another kind.
    Mismatch,
}

/// Add one record in stream order to a tree under construction.
///
/// An opener starts a scope. A closer ends the innermost scope, which then
/// enters the tree. Other records go into the innermost scope.
pub(super) fn push(
    top: &mut Vec<ModuleSymbol>,
    open: &mut Vec<Open>,
    record: SymbolRecord,
    offset: u32,
) -> Result<Pushed> {
    if opens_scope(record.data()) {
        open.try_reserve(1).map_err(|_| no_memory())?;
        open.push(Open {
            opener: record,
            children: Vec::new(),
            offset,
        });
        return Ok(Pushed::Done);
    }
    if closes_scope(record.kind()) {
        let Some(scope) = open.last() else {
            return Ok(Pushed::Mismatch);
        };
        if !matches(scope.opener.kind(), record.kind()) {
            return Ok(Pushed::Mismatch);
        }
        let Some(scope) = open.pop() else {
            return Ok(Pushed::Mismatch);
        };
        let node = ModuleSymbol::Scope(Box::new(Scope {
            opener: scope.opener,
            children: scope.children,
            closer: record,
        }));
        attach(top, open, node)?;
        return Ok(Pushed::Done);
    }
    attach(top, open, ModuleSymbol::Record(record))?;
    Ok(Pushed::Done)
}

/// Decode the symbol records of a module into a scope tree.
///
/// `base` is the module stream offset of the first record. Each record gets
/// its position in the stream as its number. Return `None` if the records do
/// not form a tree: a closer without a matching opener, an opener without a
/// closer, or a source link that differs from the tree. Such a module stays
/// opaque.
pub(super) fn decode(records: &[u8], base: u32) -> Result<Option<(Vec<ModuleSymbol>, u32)>> {
    let mut top = Vec::new();
    let mut open: Vec<Open> = Vec::new();
    let mut count = 0_u32;
    for record in crate::native::codeview::Records::new(records) {
        let record = record?;
        let symbol = Symbol::decode(record)?.into_owned();
        let offset = base.checked_add(record.offset).ok_or_else(|| {
            Error::new(ErrorKind::OffsetOverflow, None)
                .with_context("module symbol offset does not fit in 32 bits")
        })?;
        if let Some(links) = symbol.links() {
            let parent = open.last().map_or(0, |scope| scope.offset);
            if links.parent != ModuleSymbolOffset(parent) {
                return Ok(None);
            }
        }
        if closes_scope(symbol.kind())
            && let Some(scope) = open.last()
            && scope
                .opener
                .origin()
                .and_then(Symbol::links)
                .is_none_or(|links| links.end != ModuleSymbolOffset(offset))
        {
            return Ok(None);
        }
        let mut node = SymbolRecord::from_source(symbol);
        node.set_id(count);
        count = count.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("module has too many symbol records")
        })?;
        if let Pushed::Mismatch = push(&mut top, &mut open, node, offset)? {
            return Ok(None);
        }
    }
    if !open.is_empty() {
        return Ok(None);
    }
    Ok(Some((top, count)))
}

/// Call `visit` for each record of `nodes` in pre-order: a record, or the
/// opener of a scope, then its children, then its closer.
pub(super) fn walk<'n>(
    nodes: &'n [ModuleSymbol],
    visit: &mut impl FnMut(&'n SymbolRecord) -> Result<()>,
) -> Result<()> {
    for node in nodes {
        match node {
            ModuleSymbol::Record(record) => visit(record)?,
            ModuleSymbol::Scope(scope) => {
                visit(&scope.opener)?;
                walk(&scope.children, visit)?;
                visit(&scope.closer)?;
            }
        }
    }
    Ok(())
}

/// Call `visit` for each record of `nodes` in pre-order, for a change.
pub(super) fn walk_mut(
    nodes: &mut [ModuleSymbol],
    visit: &mut impl FnMut(&mut SymbolRecord) -> Result<()>,
) -> Result<()> {
    for node in nodes {
        match node {
            ModuleSymbol::Record(record) => visit(record)?,
            ModuleSymbol::Scope(scope) => {
                visit(&mut scope.opener)?;
                walk_mut(&mut scope.children, visit)?;
                visit(&mut scope.closer)?;
            }
        }
    }
    Ok(())
}

/// Return the first record of `nodes` in pre-order that `test` accepts.
pub(super) fn find<'n>(
    nodes: &'n [ModuleSymbol],
    test: &impl Fn(&SymbolRecord) -> bool,
) -> Option<&'n SymbolRecord> {
    for node in nodes {
        match node {
            ModuleSymbol::Record(record) if test(record) => return Some(record),
            ModuleSymbol::Record(_) => {}
            ModuleSymbol::Scope(scope) => {
                if test(&scope.opener) {
                    return Some(&scope.opener);
                }
                if let Some(found) = find(&scope.children, test) {
                    return Some(found);
                }
                if test(&scope.closer) {
                    return Some(&scope.closer);
                }
            }
        }
    }
    None
}

/// Return `true` if `data` is a procedure: a native procedure of any
/// architecture, or a managed procedure.
pub(super) const fn is_procedure(data: &SymbolData<'_>) -> bool {
    matches!(
        data,
        SymbolData::Procedure(_)
            | SymbolData::MipsProcedure(_)
            | SymbolData::Ia64Procedure(_)
            | SymbolData::ManagedProcedure(_)
    )
}

/// Find the scope of a procedure with the name `name` in `nodes`, at any
/// depth.
pub(super) fn find_procedure<'n>(
    nodes: &'n mut [ModuleSymbol],
    name: &[u8],
) -> Option<&'n mut Scope> {
    for node in nodes {
        if let ModuleSymbol::Scope(scope) = node {
            if is_procedure(scope.opener.data()) && scope.opener.name() == Some(name) {
                return Some(&mut **scope);
            }
            if let Some(found) = find_procedure(&mut scope.children, name) {
                return Some(found);
            }
        }
    }
    None
}
