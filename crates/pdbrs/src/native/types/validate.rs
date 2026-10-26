//! Reference validation of a TPI stream with bounded work.
//!
//! [`validate_type_references`] decodes each record once and checks each
//! type index with [`TypeLeaf::for_each_reference`]:
//!
//! - A primitive index must have a known kind and mode.
//! - A record index must be inside the stream.
//! - A record index must refer to a record kind of its
//!   [`ReferenceRole`], and [`TypeIndex::NONE`] must be valid for the role.
//!
//! A reference to the same record or to a later record is a forward
//! reference. Microsoft and LLVM writers do not write them, but the format
//! does not forbid them. With forward references, the function searches
//! the reference graph for a cycle. It also computes the longest reference
//! chain (the depth). The search is iterative. [`ReferenceLimits`] bounds
//! the depth and the number of visited references, so a hostile stream
//! cannot make the work or the memory grow without a limit.

use alloc::vec::Vec;

use super::{HEADER_SIZE, TypeTable};
use crate::native::codeview::LeafKind;
use crate::native::codeview::index::TypeIndex;
use crate::native::codeview::type_record::{ReferenceRole, TypeLeaf};
use crate::native::msf::StreamId;
use crate::{Error, ErrorKind, Result};

/// The stream index of the TPI stream.
const TPI_STREAM: u32 = 2;

/// The limits of [`validate_type_references`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ReferenceLimits {
    /// The largest valid depth: the number of records in the longest chain
    /// of record references.
    pub max_depth: u32,
    /// The largest number of references that the function visits, in all
    /// passes together.
    pub max_work: u64,
}

impl Default for ReferenceLimits {
    /// Return a depth limit of 4096 and a work limit of 2^28 references.
    fn default() -> Self {
        Self {
            max_depth: 4096,
            max_work: 1 << 28,
        }
    }
}

/// The result of [`validate_type_references`].
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ReferenceSummary {
    /// The number of type indices in all records.
    pub references: u64,
    /// The number of references to a primitive type.
    pub primitive_references: u64,
    /// The number of references to the same record or to a later record.
    pub forward_references: u64,
    /// The number of records in the longest chain of record references.
    pub max_depth: u32,
}

/// Return an error at the TPI stream location of the record at `offset`.
fn located(error: Error, offset: u32) -> Error {
    let location = u64::try_from(HEADER_SIZE)
        .unwrap_or(u64::MAX)
        .saturating_add(u64::from(offset));
    error.with_stream(StreamId(TPI_STREAM), location)
}

/// Return an invalid-reference error with `context`.
fn invalid(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidReference, None).with_context(context)
}

/// Return the error for an allocation that failed.
fn no_memory() -> Error {
    Error::new(ErrorKind::LimitExceeded, None)
        .with_context("type reference graph cannot be allocated")
}

/// The record references of all records, in compressed rows.
struct Graph {
    /// The index of the first record.
    begin: u32,
    /// The start of the targets of each record in `targets`, and the end.
    starts: Vec<u32>,
    /// The positions of the target records.
    targets: Vec<u32>,
    /// The record offsets, for error locations.
    offsets: Vec<u32>,
}

/// A counter of visited references with a limit.
struct Work {
    /// The visited references.
    done: u64,
    /// The limit.
    limit: u64,
}

impl Work {
    /// Count one visited reference.
    fn step(&mut self) -> Result<()> {
        self.done += 1;
        if self.done > self.limit {
            Err(Error::new(ErrorKind::LimitExceeded, None)
                .with_context("type reference validation exceeds its work limit"))
        } else {
            Ok(())
        }
    }
}

/// Check the references of all records of a TPI stream.
///
/// # Errors
///
/// The errors have the TPI stream location of the record.
///
/// - [`ErrorKind::InvalidReference`] if a reference breaks a rule above.
/// - [`ErrorKind::InvalidFormat`] if the references form a cycle.
/// - [`ErrorKind::LimitExceeded`] if the depth or the work exceeds `limits`,
///   or if the graph cannot be allocated.
/// - The errors of [`TypeLeaf::decode`] and of the record framing.
pub fn validate_type_references(
    table: &TypeTable<'_>,
    limits: ReferenceLimits,
) -> Result<ReferenceSummary> {
    let mut summary = ReferenceSummary::default();
    let mut work = Work {
        done: 0,
        limit: limits.max_work,
    };
    let graph = build_graph(table, &mut summary, &mut work)?;
    summary.max_depth = if summary.forward_references == 0 {
        ordered_depth(&graph, limits, &mut work)?
    } else {
        searched_depth(&graph, limits, &mut work)?
    };
    Ok(summary)
}

/// Decode all records, check each reference and keep the record references.
fn build_graph(
    table: &TypeTable<'_>,
    summary: &mut ReferenceSummary,
    work: &mut Work,
) -> Result<Graph> {
    let header = table.stream().header();
    let count = table.len();
    let mut graph = Graph {
        begin: header.index_begin,
        starts: Vec::new(),
        targets: Vec::new(),
        offsets: Vec::new(),
    };
    graph
        .starts
        .try_reserve_exact(count + 1)
        .map_err(|_| no_memory())?;
    graph
        .offsets
        .try_reserve_exact(count)
        .map_err(|_| no_memory())?;
    graph.starts.push(0);
    for record in table.stream().records() {
        let record = record?;
        let offset = record.record.offset;
        graph.offsets.push(offset);
        let leaf = TypeLeaf::decode(record.record).map_err(|error| located(error, offset))?;
        leaf.for_each_reference(|reference| {
            work.step()?;
            summary.references += 1;
            let index = reference.index;
            if index == TypeIndex::NONE && !reference.role.allows_none() {
                return Err(invalid("type reference of this role is not T_NOTYPE"));
            }
            if index.is_primitive() {
                index
                    .primitive()
                    .map_err(|_| invalid("type reference is an unknown primitive type"))?;
                if !matches!(reference.role, ReferenceRole::Type) && index != TypeIndex::NONE {
                    return Err(invalid("type reference of this role is a primitive type"));
                }
                summary.primitive_references += 1;
                return Ok(());
            }
            let target = table.record(index)?;
            if let Some(kinds) = reference.role.kinds()
                && !kinds.contains(&LeafKind(target.record.kind))
            {
                return Err(invalid(
                    "type reference refers to a record of the wrong kind",
                ));
            }
            if index.0 >= record.index {
                summary.forward_references += 1;
            }
            graph.targets.try_reserve(1).map_err(|_| no_memory())?;
            graph.targets.push(index.0 - graph.begin);
            Ok(())
        })
        .map_err(|error| located(error, offset))?;
        let end = u32::try_from(graph.targets.len()).map_err(|_| no_memory())?;
        graph.starts.push(end);
    }
    Ok(graph)
}

impl Graph {
    /// Return the targets of the record at `position`.
    fn targets(&self, position: usize) -> &[u32] {
        let start = self.starts[position] as usize;
        let end = self.starts[position + 1] as usize;
        &self.targets[start..end]
    }

    /// Return the depth error at the record at `position`.
    fn too_deep(&self, position: usize) -> Error {
        located(
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("type reference chain exceeds the depth limit"),
            self.offsets[position],
        )
    }
}

/// Compute the depth when each reference refers to an earlier record.
fn ordered_depth(graph: &Graph, limits: ReferenceLimits, work: &mut Work) -> Result<u32> {
    let mut depths: Vec<u32> = Vec::new();
    depths
        .try_reserve_exact(graph.offsets.len())
        .map_err(|_| no_memory())?;
    let mut deepest = 0;
    for position in 0..graph.offsets.len() {
        let mut depth = 1;
        for &target in graph.targets(position) {
            work.step()?;
            depth = depth.max(depths[target as usize] + 1);
        }
        if depth > limits.max_depth {
            return Err(graph.too_deep(position));
        }
        deepest = deepest.max(depth);
        depths.push(depth);
    }
    Ok(deepest)
}

/// The search state of a record.
#[derive(Clone, Copy, Eq, PartialEq)]
enum State {
    /// Not visited.
    New,
    /// On the search stack.
    Open,
    /// Visited, with its depth.
    Done(u32),
}

/// Search the graph for a cycle and compute the depth.
fn searched_depth(graph: &Graph, limits: ReferenceLimits, work: &mut Work) -> Result<u32> {
    let count = graph.offsets.len();
    let mut states = Vec::new();
    states.try_reserve_exact(count).map_err(|_| no_memory())?;
    states.resize(count, State::New);
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut deepest = 0;
    for root in 0..count {
        if states[root] != State::New {
            continue;
        }
        states[root] = State::Open;
        stack.try_reserve(1).map_err(|_| no_memory())?;
        stack.push((root, 0));
        while let Some(&mut (position, ref mut next)) = stack.last_mut() {
            let targets = graph.targets(position);
            if let Some(&target) = targets.get(*next) {
                *next += 1;
                work.step()?;
                let target = target as usize;
                match states[target] {
                    State::Open => {
                        return Err(located(
                            Error::new(ErrorKind::InvalidFormat, None)
                                .with_context("type references form a cycle"),
                            graph.offsets[position],
                        ));
                    }
                    State::New => {
                        if stack.len() >= usize::try_from(limits.max_depth).unwrap_or(usize::MAX) {
                            return Err(graph.too_deep(target));
                        }
                        states[target] = State::Open;
                        stack.try_reserve(1).map_err(|_| no_memory())?;
                        stack.push((target, 0));
                    }
                    State::Done(_) => {}
                }
                continue;
            }
            let depth = 1 + targets
                .iter()
                .map(|&target| match states[target as usize] {
                    State::Done(depth) => depth,
                    State::New | State::Open => 0,
                })
                .max()
                .unwrap_or(0);
            if depth > limits.max_depth {
                return Err(graph.too_deep(position));
            }
            deepest = deepest.max(depth);
            states[position] = State::Done(depth);
            stack.pop();
        }
    }
    Ok(deepest)
}
