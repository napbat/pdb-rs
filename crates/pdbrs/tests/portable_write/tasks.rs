//! The editing tasks of #41: `set_source_link` and `embed_source`.

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
