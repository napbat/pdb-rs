//! The tests of the dump mapping.

use std::collections::BTreeSet;

use super::{
    Removal, Space, first_difference, grouped, index_width, map_dump, reference_tokens, tokens,
};

/// A TPI section of five records, an IPI section and two symbol sections.
const SOURCE: &str = "\
                     Types (TPI Stream)
============================================================
  Showing 5 records
  0x1000 | LF_ARGLIST [size = 8]
  0x1001 | LF_PROCEDURE [size = 16]
           return type = 0x0074 (int), # args = 0, param list = 0x1000
           calling conv = cdecl, options = None
  0x1002 | LF_MODIFIER [size = 12]
           referent = 0x0074 (int), modifiers = const
  0x1003 | LF_POINTER [size = 12]
           referent = 0x1001, mode = pointer, opts = None, kind = ptr64
  0x1004 | LF_MODIFIER [size = 12]
           referent = 0x1003, modifiers = const

                     Types (IPI Stream)
============================================================
  Showing 4 records
  0x1000 | LF_STRING_ID [size = 16] ID: <no type>, String: demo
  0x1001 | LF_FUNC_ID [size = 28]
           name = main, type = 0x1003, parent scope = 0x1000
  0x1002 | LF_MFUNC_ID [size = 28]
           name = twice, type = 0x1003, class type = 0x1003
  0x1003 | LF_UDT_SRC_LINE [size = 16]
           udt = 0x1003, file = 4100, line = 20

                          Symbols
============================================================
  Mod 0000 | `main.obj`:
     204 | S_GPROC32 [size = 44] `main`
           type = `0x1003 (int ())`, debug start = 19, debug end = 330, flags = none
     292 | S_REGREL32 [size = 24] `record`
           type = 0x1003 (int (*)()), register = RSP, offset = 56
     372 | S_BUILDINFO [size = 8] BuildId = `0x1003`
     380 | S_GPROC32_ID [size = 44] `f`
           type = `0x1003 (<unknown UDT>)`, debug start = 0, debug end = 7, flags = none
     420 | S_INLINESITE [size = 20]
           inlinee = 0x1003 (main), parent = 408, end = 788
     440 | S_LOCAL [size = 16] `a`
           type=0x1003 (int (*)()), flags = param
     460 | S_CALLEES [size = 8]
           callee: 0x1003 (f)

                       Global Symbols
============================================================
  Records
   15488 | S_UDT [size = 16] `?A0x71023ff6::fn`
           original type = 0x1003 (int (*)())
";

/// The source text after the removal of `0x1002` and `0x1004`.
const EXPECTED: &str = "\
                     Types (TPI Stream)
============================================================
  Showing 3 records
  0x1000 | LF_ARGLIST [size = 8]
  0x1001 | LF_PROCEDURE [size = 16]
           return type = 0x0074 (int), # args = 0, param list = 0x1000
           calling conv = cdecl, options = None
  0x1002 | LF_POINTER [size = 12]
           referent = 0x1001, mode = pointer, opts = None, kind = ptr64

                     Types (IPI Stream)
============================================================
  Showing 4 records
  0x1000 | LF_STRING_ID [size = 16] ID: <no type>, String: demo
  0x1001 | LF_FUNC_ID [size = 28]
           name = main, type = 0x1002, parent scope = 0x1000
  0x1002 | LF_MFUNC_ID [size = 28]
           name = twice, type = 0x1002, class type = 0x1002
  0x1003 | LF_UDT_SRC_LINE [size = 16]
           udt = 0x1002, file = 4100, line = 20

                          Symbols
============================================================
  Mod 0000 | `main.obj`:
     204 | S_GPROC32 [size = 44] `main`
           type = `0x1002 (int ())`, debug start = 19, debug end = 330, flags = none
     292 | S_REGREL32 [size = 24] `record`
           type = 0x1002 (int (*)()), register = RSP, offset = 56
     372 | S_BUILDINFO [size = 8] BuildId = `0x1003`
     380 | S_GPROC32_ID [size = 44] `f`
           type = `0x1003 (<unknown UDT>)`, debug start = 0, debug end = 7, flags = none
     420 | S_INLINESITE [size = 20]
           inlinee = 0x1003 (main), parent = 408, end = 788
     440 | S_LOCAL [size = 16] `a`
           type=0x1002 (int (*)()), flags = param
     460 | S_CALLEES [size = 8]
           callee: 0x1003 (f)

                       Global Symbols
============================================================
  Records
   15488 | S_UDT [size = 16] `?A0x71023ff6::fn`
           original type = 0x1002 (int (*)())
";

/// The output index of the removal of `0x1002` and `0x1004` from five records.
fn compact(value: u32) -> Option<u32> {
    match value {
        0x1000 | 0x1001 => Some(value),
        0x1003 => Some(0x1002),
        _ => None,
    }
}

#[test]
fn removes_type_blocks_and_maps_only_type_fields() {
    let removed = BTreeSet::from([0x1002, 0x1004]);
    let removal = Removal {
        space: Space::Types,
        removed: &removed,
        map: &compact,
    };
    let mapped = map_dump(SOURCE, &removal).unwrap();
    assert_eq!(first_difference(EXPECTED, &mapped), None);
    assert_eq!(mapped, EXPECTED);
}

#[test]
fn rejects_a_reference_to_a_removed_record() {
    let removed = BTreeSet::from([0x1001]);
    let map = |value: u32| (value != 0x1001).then_some(value);
    let removal = Removal {
        space: Space::Types,
        removed: &removed,
        map: &map,
    };
    let error = map_dump(SOURCE, &removal).unwrap_err();
    assert!(error.contains("index 0x1001: referent"), "{error}");
}

#[test]
fn changes_the_index_columns_with_the_largest_index() {
    // 0x1000 + 5_904 is 10_000; 0x1000 + 5_903 has 4 decimal digits.
    assert_eq!(index_width(5_904), 7);
    assert_eq!(index_width(5_903), 6);
    // A list entry aligns to the decimal digits of the largest index of the list:
    // 0x2710 is 10_000, 0x270F is 9_999.
    let source = "\
                     Types (TPI Stream)
  Showing 5,904 records
   0x1000 | LF_MODIFIER [size = 12]
            referent = 0x1001, modifiers = const
   0x1001 | LF_ARGLIST [size = 12]
             0x2710: `int*`
            0x0074 (int): `int`
   0x1002 | LF_POINTER [size = 12]
            referent = 0x2710, mode = pointer, opts = None, kind = ptr64
";
    let removed = BTreeSet::from([0x1000]);
    let map = |value: u32| value.checked_sub(1);
    let removal = Removal {
        space: Space::Types,
        removed: &removed,
        map: &map,
    };
    let mapped = map_dump(source, &removal).unwrap();
    let expected = "\
                     Types (TPI Stream)
  Showing 5,903 records
  0x1000 | LF_ARGLIST [size = 12]
           0x270F: `int*`
           0x0074 (int): `int`
  0x1001 | LF_POINTER [size = 12]
           referent = 0x270F, mode = pointer, opts = None, kind = ptr64
";
    assert_eq!(mapped, expected);
}

#[test]
fn finds_reference_tokens_after_the_type_section() {
    // The record header indices of the IPI section are not references.
    let found = reference_tokens(SOURCE);
    assert_eq!(found.all, BTreeSet::from([0x1000, 0x1003]));
    assert_eq!(found.ids, BTreeSet::from([0x1000, 0x1003]));
    // A value in a type field is not an ID reference.
    let types = "\
                          Symbols
     292 | S_REGREL32 [size = 24] `record`
           type = 0x1003 (int (*)()), register = RSP, offset = 56
     420 | S_INLINESITE [size = 20]
           inlinee = 0x1004 (main), parent = 408, end = 788
";
    let found = reference_tokens(types);
    assert_eq!(found.all, BTreeSet::from([0x1003, 0x1004]));
    assert_eq!(found.ids, BTreeSet::from([0x1004]));
}

#[test]
fn reads_tokens_and_groups() {
    let values: Vec<u32> = tokens("a 0x1A2B, ?A0x7102 0x12z 0xFF`")
        .iter()
        .map(|token| token.value)
        .collect();
    assert_eq!(values, [0x1A2B, 0xFF]);
    assert_eq!(grouped(224_838), "224,838");
    assert_eq!(grouped(880), "880");
    assert_eq!(grouped(1_000), "1,000");
    assert_eq!(
        first_difference("a\nb\n", "a\n").as_deref(),
        Some("line 2: expected \"b\", found \"(end of the text)\"")
    );
}

/// The source of `removes_id_blocks_and_maps_only_id_fields` after the removal of the ID
/// `0x1002`.
const EXPECTED_IDS: &str = "\
                     Types (IPI Stream)
============================================================
  Showing 4 records
  0x1000 | LF_STRING_ID [size = 16] ID: <no type>, String: demo 0x1003
  0x1001 | LF_FUNC_ID [size = 28]
           name = main, type = 0x1003, parent scope = 0x1000
  0x1002 | LF_UDT_SRC_LINE [size = 16]
           udt = 0x1003, file = 4100, line = 20
  0x1003 | LF_BUILDINFO [size = 16]
           0x1000: `C:\\src`
           0x1002: `0x1003`

                          Symbols
============================================================
  Mod 0000 | `main.obj`:
     204 | S_GPROC32 [size = 44] `main`
           type = `0x1003 (int ())`, debug start = 19, debug end = 330, flags = none
     372 | S_BUILDINFO [size = 8] BuildId = `0x1002`
     380 | S_GPROC32_ID [size = 44] `f`
           type = `0x1002 (<unknown UDT>)`, debug start = 0, debug end = 7, flags = none
     420 | S_INLINESITE [size = 20]
           inlinee = 0x1002 (main), parent = 408, end = 788
     460 | S_CALLEES [size = 8]
           callee: 0x1002 (f)
";

#[test]
fn removes_id_blocks_and_maps_only_id_fields() {
    let source = "\
                     Types (IPI Stream)
============================================================
  Showing 5 records
  0x1000 | LF_STRING_ID [size = 16] ID: <no type>, String: demo 0x1003
  0x1001 | LF_FUNC_ID [size = 28]
           name = main, type = 0x1003, parent scope = 0x1000
  0x1002 | LF_UDT_SRC_LINE [size = 16]
           udt = 0x1001, file = 4100, line = 10
  0x1003 | LF_UDT_SRC_LINE [size = 16]
           udt = 0x1003, file = 4100, line = 20
  0x1004 | LF_BUILDINFO [size = 16]
           0x1000: `C:\\src`
           0x1003: `0x1003`

                          Symbols
============================================================
  Mod 0000 | `main.obj`:
     204 | S_GPROC32 [size = 44] `main`
           type = `0x1003 (int ())`, debug start = 19, debug end = 330, flags = none
     372 | S_BUILDINFO [size = 8] BuildId = `0x1003`
     380 | S_GPROC32_ID [size = 44] `f`
           type = `0x1003 (<unknown UDT>)`, debug start = 0, debug end = 7, flags = none
     420 | S_INLINESITE [size = 20]
           inlinee = 0x1003 (main), parent = 408, end = 788
     460 | S_CALLEES [size = 8]
           callee: 0x1003 (f)
";
    let removed = BTreeSet::from([0x1002]);
    let map = |value: u32| match value {
        0x1000 | 0x1001 => Some(value),
        0x1003 | 0x1004 => Some(value - 1),
        _ => None,
    };
    let removal = Removal {
        space: Space::Ids,
        removed: &removed,
        map: &map,
    };
    let mapped = map_dump(source, &removal).unwrap();
    assert_eq!(first_difference(EXPECTED_IDS, &mapped), None);
    assert_eq!(mapped, EXPECTED_IDS);
}
