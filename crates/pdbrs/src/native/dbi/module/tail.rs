//! The global reference region of a module stream, read without the other
//! regions (#48).

use alloc::vec::Vec;

use super::global_refs::GlobalRefs;
use super::{Module, SIGNATURE_SIZE, module_error, region};
use crate::io::ReadAt;
use crate::native::output::no_memory;
use crate::native::symbols::SymbolOffset;
use crate::{Error, ErrorKind, Result};

/// The global references of a module stream and their position.
#[derive(Debug)]
pub(crate) struct GlobalRefRegion {
    /// The module stream offset of the first reference.
    pub(crate) start: usize,
    /// The references in stream order.
    pub(crate) offsets: Vec<SymbolOffset>,
}

/// Read the global references of the module stream `reader` of `module`.
///
/// The region sizes of `module` give the position of the region. The
/// function reads only the size field and the references, with one read. It
/// checks the bounds of each region and the size field with the rules of
/// [`super::ModuleStream::parse`]. It does not read the symbol region, so it
/// does not check the symbol signature.
///
/// # Errors
///
/// - [`ErrorKind::MissingStream`] if `module` has no module stream.
/// - The region errors of [`super::ModuleStream::parse`] and
///   [`super::ModuleStream::global_refs`].
/// - The errors of `reader`.
pub(crate) fn read_global_refs<R: ReadAt + ?Sized>(
    reader: &R,
    module: &Module<'_>,
) -> Result<GlobalRefRegion> {
    let stream = module.stream.ok_or_else(|| {
        Error::new(ErrorKind::MissingStream, None).with_context("module has no module stream")
    })?;
    let id = stream.0;
    let length = usize::try_from(reader.len()?).map_err(|_| {
        module_error(
            id,
            ErrorKind::LimitExceeded,
            0,
            "module stream does not fit in memory",
        )
    })?;
    let symbols = region(
        id,
        0,
        module.symbol_bytes,
        length,
        "module symbol region is truncated",
    )?;
    if !symbols.is_empty() && symbols.len() < SIGNATURE_SIZE {
        return Err(module_error(
            id,
            ErrorKind::InvalidFormat,
            0,
            "module symbol region is smaller than its signature",
        ));
    }
    let c11 = region(
        id,
        symbols.end,
        module.c11_bytes,
        length,
        "module C11 line region is truncated",
    )?;
    let c13 = region(
        id,
        c11.end,
        module.c13_bytes,
        length,
        "module C13 line region is truncated",
    )?;
    let tail_length = length - c13.end;
    if tail_length < 4 {
        return Err(module_error(
            id,
            ErrorKind::UnexpectedEof,
            length,
            "module global reference size is truncated",
        ));
    }
    let mut tail = Vec::new();
    tail.try_reserve_exact(tail_length)
        .map_err(|_| no_memory())?;
    tail.resize(tail_length, 0);
    reader.read_exact_at(c13.end as u64, &mut tail)?;
    let (size, refs) = tail.split_at(4);
    let size = u32::from_le_bytes([size[0], size[1], size[2], size[3]]);
    let start = c13.end + 4;
    let range = region(
        id,
        start,
        size,
        length,
        "module global references are truncated",
    )?;
    if range.end != length {
        return Err(module_error(
            id,
            ErrorKind::InvalidFormat,
            range.end,
            "module stream has bytes after the global references",
        ));
    }
    let parsed = GlobalRefs::parse(refs, id, start)?;
    let mut offsets = Vec::new();
    offsets
        .try_reserve_exact(parsed.len())
        .map_err(|_| no_memory())?;
    offsets.extend(parsed.iter());
    Ok(GlobalRefRegion { start, offsets })
}
