//! The external dependencies of a PDB: type servers, precompiled types and FASTLINK objects.

use pdbrs::io::ReadAt;
use pdbrs::native::Pdb;

use crate::{Outcome, Report};

/// Count the external dependencies. The example does not open the external files.
pub(crate) fn external_dependencies(pdb: &Pdb<impl ReadAt>, report: &mut Report) -> Outcome {
    let dependencies = pdb.external_dependencies()?;
    if dependencies.mini_pdb && dependencies.fastlink_objects.is_empty() {
        report.inconsistency("FASTLINK PDB without S_FASTLINK records".to_owned());
    }
    report.add("external type servers", dependencies.type_servers.len());
    report.add(
        "external precompiled-type groups",
        dependencies.precompiled.len(),
    );
    report.add(
        "external precompiled-type modules",
        dependencies
            .precompiled
            .iter()
            .map(|group| group.modules.len())
            .sum(),
    );
    report.add(
        "external FASTLINK object files",
        dependencies.fastlink_objects.len(),
    );
    if dependencies.no_type_merge {
        report.add("PDBs without type merging (NOTM)", 1);
    }
    report.add(
        "modules with S_MOD_TYPEREF type records",
        dependencies.module_types.len(),
    );
    report.add(
        "modules with precompiled types in an object file (external)",
        dependencies.object_precompiled_types.len(),
    );
    Ok(())
}
