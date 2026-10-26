//! File output through a temporary sibling file.

use std::fs;
use std::path::PathBuf;

use pdbrs::ErrorKind;
use pdbrs::portable::Pdb;

use crate::{parts_with_methods, plan};

/// Make an empty directory for one test below the target directory.
fn directory(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("portable-write-{name}"));
    drop(fs::remove_dir_all(&path));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn write_file_replaces_the_destination_and_leaves_no_temporary_file() {
    let directory = directory("write-file");
    let path = directory.join("out.pdb");
    fs::write(&path, b"old contents").unwrap();
    let plan = plan(parts_with_methods(1));
    plan.write_file(&path).unwrap();
    assert_eq!(fs::read(&path).unwrap(), plan.to_vec().unwrap());
    let entries = fs::read_dir(&directory).unwrap().count();
    assert_eq!(entries, 1);
}

#[test]
fn write_file_error_does_not_make_a_file() {
    let directory = directory("write-file-error");
    let path = directory.join("missing").join("out.pdb");
    let error = plan(parts_with_methods(1)).write_file(&path).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    assert!(!path.exists());
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
}

#[test]
fn copy_exact_file_copies_the_source() {
    let directory = directory("copy-exact-file");
    let bytes = plan(parts_with_methods(1)).to_vec().unwrap();
    let source = directory.join("source.pdb");
    fs::write(&source, &bytes).unwrap();
    let pdb = Pdb::open(&source).unwrap();
    let path = directory.join("copy.pdb");
    assert_eq!(
        pdb.copy_exact_file(&path).unwrap(),
        u64::try_from(bytes.len()).unwrap()
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
}
