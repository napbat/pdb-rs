//! The editing tasks `set_source_link` and `embed_source`, and the source path remap
//! `remap_source_paths`.

use pdbrs::ErrorKind;
use pdbrs::portable::custom::{CustomDebugInformation, CustomDebugKind, CustomDebugParent};
use pdbrs::portable::embedded::{EmbeddedFormat, EmbeddedSource};
use pdbrs::portable::metadata::{DocumentId, MetadataStream, MetadataToken, TableId};
use pdbrs::portable::{IdentityPolicy, Pdb, PdbEditor, StreamChange};

use crate::build::{information, sample, write};
use crate::open;

/// A Source Link file.
const SOURCE_LINK: &[u8] = br#"{"documents":{"C:\\src\\*":"https://example.invalid/src/*"}}"#;
/// A source text that DEFLATE makes smaller.
const SOURCE: &[u8] = b"class Program { static void Main() { } } // class Program class Program";

/// Return the parent of the Source Link record: the module.
fn module() -> CustomDebugParent {
    CustomDebugParent::Metadata(MetadataToken::new(TableId::MODULE, 1).unwrap())
}

/// Return the values of the records of `pdb` with `parent` and `kind`.
fn values(pdb: &Pdb<&[u8]>, parent: CustomDebugParent, kind: CustomDebugKind) -> Vec<Vec<u8>> {
    pdb.custom_debug_informations()
        .map(|information| information.unwrap().1)
        .filter(|information| information.parent == parent && information.kind == kind)
        .map(|information| information.value.to_vec())
        .collect()
}

#[test]
fn set_source_link_adds_and_replaces_the_module_record() {
    let source = write(&sample());
    let pdb = open(&source);
    let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    editor.set_source_link(SOURCE_LINK.to_vec()).unwrap();
    let plan = editor
        .plan(pdbrs::portable::EditOptions::default())
        .unwrap();
    assert_eq!(
        plan.report().stream(MetadataStream::Strings),
        Some(StreamChange::Copied)
    );
    let added = plan.to_vec().unwrap();
    let added = open(&added);
    let kind = CustomDebugKind::SOURCE_LINK;
    assert_eq!(values(&added, module(), kind), [SOURCE_LINK.to_vec()]);

    let mut editor = added.edit(IdentityPolicy::ContentDerived);
    let replaced = b"{\"documents\":{}}".to_vec();
    editor.set_source_link(replaced.clone()).unwrap();
    let edited = write(&editor);
    let edited = open(&edited);
    assert_eq!(values(&edited, module(), kind), [replaced]);
    assert_eq!(
        edited.custom_debug_informations().count(),
        added.custom_debug_informations().count()
    );
}

#[test]
fn embed_source_adds_and_replaces_the_document_record() {
    let source = write(&sample());
    let pdb = open(&source);
    let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    editor
        .embed_source(DocumentId(1), SOURCE, EmbeddedFormat::Deflate)
        .unwrap();
    let added = write(&editor);
    let added = open(&added);
    let parent = CustomDebugParent::Document(DocumentId(1));
    let kind = CustomDebugKind::EMBEDDED_SOURCE;
    let [blob] = values(&added, parent, kind).try_into().unwrap();
    let embedded = EmbeddedSource::parse(&blob).unwrap();
    assert_eq!(embedded.format(), EmbeddedFormat::Deflate);
    assert_eq!(&*embedded.decode(usize::MAX).unwrap(), SOURCE);

    let mut editor = added.edit(IdentityPolicy::ContentDerived);
    editor
        .embed_source(DocumentId(1), SOURCE, EmbeddedFormat::Raw)
        .unwrap();
    let edited = write(&editor);
    let edited = open(&edited);
    let [blob] = values(&edited, parent, kind).try_into().unwrap();
    let embedded = EmbeddedSource::parse(&blob).unwrap();
    assert_eq!(embedded.format(), EmbeddedFormat::Raw);
    assert_eq!(&*embedded.decode(usize::MAX).unwrap(), SOURCE);
}

/// Apply `task` to an editor with an embedded source. Check that it fails with `kind` and
/// that the output stays the same.
fn rejected<'p>(
    pdb: &'p Pdb<&'p [u8]>,
    kind: ErrorKind,
    task: impl FnOnce(&mut PdbEditor<'p, &'p [u8]>) -> pdbrs::Result<()>,
) {
    let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    editor
        .embed_source(DocumentId(2), SOURCE, EmbeddedFormat::Raw)
        .unwrap();
    let before = write(&editor);
    let error = task(&mut editor).expect_err("rejected task");
    assert_eq!(error.kind(), kind, "{error}");
    assert_eq!(write(&editor), before, "the model changed: {error}");
}

#[test]
fn a_task_with_invalid_input_leaves_the_model_unchanged() {
    let mut twice = sample();
    let custom = twice.custom_debug_information_mut().unwrap();
    for value in [b"{}".to_vec(), b"{ }".to_vec()] {
        custom
            .push(information(module(), CustomDebugKind::SOURCE_LINK, value))
            .unwrap();
    }
    let mut document = CustomDebugInformation::new(
        CustomDebugParent::Document(DocumentId(1)),
        CustomDebugKind::EMBEDDED_SOURCE,
    );
    document.value = vec![0, 0, 0, 0].into();
    custom.push(document.clone()).unwrap();
    custom.push(document).unwrap();
    let source = write(&twice);
    let pdb = open(&source);
    rejected(&pdb, ErrorKind::InvalidFormat, |editor| {
        editor.set_source_link(b"[]".to_vec())
    });
    rejected(&pdb, ErrorKind::InvalidFormat, |editor| {
        editor.set_source_link(vec![b'{', 0xFF, b'}'])
    });
    // The module has two Source Link records, and document 1 has two embedded sources.
    rejected(&pdb, ErrorKind::InvalidFormat, |editor| {
        editor.set_source_link(SOURCE_LINK.to_vec())
    });
    rejected(&pdb, ErrorKind::InvalidFormat, |editor| {
        editor.embed_source(DocumentId(1), SOURCE, EmbeddedFormat::Raw)
    });
    rejected(&pdb, ErrorKind::InvalidReference, |editor| {
        editor.embed_source(DocumentId(3), SOURCE, EmbeddedFormat::Deflate)
    });
}

/// Replace the prefix `C:\src\` with `/_/`.
fn remap(path: &[u8]) -> Option<Vec<u8>> {
    path.strip_prefix(b"C:\\src\\".as_slice())
        .map(|rest| [b"/_/".as_slice(), rest].concat())
}

/// Return the document names of `pdb` in row order.
fn document_names(pdb: &Pdb<&[u8]>) -> Vec<Vec<u8>> {
    pdb.documents()
        .map(|document| document.unwrap().1.name.into_vec())
        .collect()
}

/// Return the sequence point blob of each method of `pdb`.
fn sequence_points(pdb: &Pdb<&[u8]>) -> Vec<Vec<u8>> {
    pdb.method_debug_informations()
        .map(|method| method.unwrap().1.sequence_points.into_vec())
        .collect()
}

#[test]
fn remap_source_paths_gives_documents_new_names() {
    let source = write(&sample());
    let pdb = open(&source);
    let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    let mut seen = Vec::new();
    let changed = editor
        .remap_source_paths(|path| {
            seen.push(path.to_vec());
            remap(path)
        })
        .unwrap();
    assert_eq!(changed, 1);
    // Each distinct name once, in byte order.
    assert_eq!(
        seen,
        [b"/src/Other.cs".to_vec(), b"C:\\src\\Program.cs".to_vec()]
    );
    let edited = write(&editor);
    let edited = open(&edited);
    assert_eq!(
        document_names(&edited),
        [b"/_/Program.cs".to_vec(), b"/src/Other.cs".to_vec()]
    );
    assert_eq!(sequence_points(&edited), sequence_points(&pdb));
}

#[test]
fn remap_source_paths_that_keeps_every_name_gives_the_source_bytes() {
    let source = write(&sample());
    let pdb = open(&source);
    let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    assert_eq!(editor.remap_source_paths(|_| None).unwrap(), 0);
    let plan = editor
        .plan(pdbrs::portable::EditOptions::default())
        .unwrap();
    assert!(
        plan.report()
            .streams()
            .iter()
            .all(|(_, change)| *change == StreamChange::Copied)
    );
    assert_eq!(plan.to_vec().unwrap(), source);
}

#[test]
fn remap_source_paths_with_an_invalid_new_name_leaves_the_model_unchanged() {
    let source = write(&sample());
    let pdb = open(&source);
    for new in [Vec::new(), b"/_/\0x.cs".to_vec(), vec![b'/', 0xFF]] {
        rejected(&pdb, ErrorKind::InvalidFormat, |editor| {
            editor
                .remap_source_paths(|path| remap(path).map(|_| new.clone()))
                .map(|_| ())
        });
    }
}
