//! One synthetic payload for each symbol kind that the crate decodes.

/// A builder for little-endian symbol payloads.
#[derive(Clone, Default)]
pub struct Payload(pub Vec<u8>);

impl Payload {
    pub fn u8(mut self, value: u8) -> Self {
        self.0.push(value);
        self
    }

    pub fn u16(mut self, value: u16) -> Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn u32(mut self, value: u32) -> Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn u64(mut self, value: u64) -> Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn bytes(mut self, value: &[u8]) -> Self {
        self.0.extend_from_slice(value);
        self
    }

    /// Append a name and its NUL terminator.
    pub fn name(self, value: &[u8]) -> Self {
        self.bytes(value).u8(0)
    }

    /// Append `count` zero bytes.
    pub fn zeros(self, count: usize) -> Self {
        self.bytes(&vec![0; count])
    }
}

/// A record kind, a valid payload and the length of its fixed fields.
///
/// A payload shorter than `fixed` bytes is not valid.
pub struct Case {
    pub kind: u16,
    pub payload: Vec<u8>,
    pub fixed: usize,
}

/// Add one case for each kind in `kinds`.
fn add(cases: &mut Vec<Case>, kinds: &[u16], payload: &Payload, fixed: usize) {
    for &kind in kinds {
        cases.push(Case {
            kind,
            payload: payload.0.clone(),
            fixed,
        });
    }
}

/// A def-range address range: offset, section and length.
fn range(payload: Payload) -> Payload {
    payload.u32(0x1000).u16(1).u16(0x20)
}

/// Return the cases of all decoded kinds.
pub fn all_kinds() -> Vec<Case> {
    let mut cases = Vec::new();
    procedures(&mut cases);
    scopes_and_calls(&mut cases);
    variables(&mut cases);
    def_ranges(&mut cases);
    data_and_references(&mut cases);
    module_records(&mut cases);
    cases
}

/// Procedure starts of all architectures.
fn procedures(cases: &mut Vec<Case>) {
    let header = Payload::default()
        .u32(0)
        .u32(0x80)
        .u32(0)
        .u32(0x40)
        .u32(4)
        .u32(0x3c);
    let classic = header.clone().u32(0x1001).u32(0x10).u16(1).u8(0x80);
    add(
        cases,
        &[0x110f, 0x1110, 0x1146, 0x1147, 0x1155, 0x1156],
        &classic.clone().name(b"proc"),
        35,
    );
    add(
        cases,
        &[0x1172, 0x1173, 0x1174, 0x1175],
        &classic.u8(0b101).name(b"proc"),
        36,
    );
    let mips = header
        .clone()
        .u32(0xff)
        .u32(0xf)
        .u32(8)
        .u32(16)
        .u32(0x1001)
        .u32(0x10)
        .u16(1)
        .u8(2)
        .u8(30)
        .name(b"mips");
    add(cases, &[0x1114, 0x1115, 0x1148, 0x1149], &mips, 52);
    let ia64 = header
        .u32(0x1001)
        .u32(0x10)
        .u16(1)
        .u16(8)
        .u8(1)
        .name(b"ia64");
    add(cases, &[0x1118, 0x1119, 0x114a, 0x114b], &ia64, 37);
    let managed = Payload::default()
        .zeros(24)
        .u32(0x0600_0001)
        .u32(0)
        .u16(1)
        .u8(0)
        .u16(0)
        .name(b"M");
    add(cases, &[0x112a, 0x112b], &managed, 37);
    let frame = Payload::default()
        .u32(0x28)
        .u32(0)
        .u32(0)
        .u32(8)
        .u32(0)
        .u16(0)
        .u32(0x0001_4200);
    add(cases, &[0x1012], &frame, 26);
}

/// Scope records, ends, inline sites and call records.
fn scopes_and_calls(cases: &mut Vec<Case>) {
    let block = Payload::default()
        .u32(4)
        .u32(0x40)
        .u32(8)
        .u32(0x10)
        .u16(1)
        .name(b"b");
    add(cases, &[0x1103, 0x1104], &block, 18);
    let separated = Payload::default()
        .u32(4)
        .u32(0x40)
        .u32(8)
        .u32(3)
        .u32(0x200)
        .u32(0x10)
        .u16(2)
        .u16(1);
    add(cases, &[0x1132], &separated, 28);
    add(cases, &[0x0006, 0x114f, 0x114e], &Payload::default(), 0);
    let label = Payload::default().u32(0x10).u16(1).u8(0).name(b"L");
    add(cases, &[0x1105], &label, 7);
    let thunk = Payload::default()
        .u32(0)
        .u32(0x40)
        .u32(0)
        .u32(0x10)
        .u16(1)
        .u16(5)
        .u8(0)
        .name(b"t");
    add(cases, &[0x1102], &thunk, 21);
    let annotations = [0x0b, 0x0c, 0x06, 0x21];
    let site = Payload::default().u32(4).u32(0x40).u32(0x1001);
    add(cases, &[0x114d], &site.clone().bytes(&annotations), 12);
    add(cases, &[0x115d], &site.u32(7).bytes(&annotations), 16);
    let ids = Payload::default().u32(2).u32(0x1001).u32(0x1002);
    add(cases, &[0x1168], &ids, 12);
    add(cases, &[0x115a, 0x115b], &ids.u32(9), 12);
    let call = Payload::default().u32(0x10).u16(1).u16(0).u32(0x1003);
    add(cases, &[0x1139], &call, 12);
    let table = Payload::default()
        .u32(0x10)
        .u16(1)
        .u16(4)
        .u32(0x20)
        .u32(0x30)
        .u16(1)
        .u16(2)
        .u32(5);
    add(cases, &[0x1159], &table, 24);
    let profile = Payload::default().u32(3).u64(100).u32(20).u32(18);
    add(cases, &[0x115c], &profile, 20);
    add(
        cases,
        &[0x1169],
        &Payload::default().u32(0x1001).name(b"hot"),
        4,
    );
    add(
        cases,
        &[0x117c],
        &Payload::default().u16(1).u32(0x10).u16(1),
        8,
    );
    add(
        cases,
        &[0x117d],
        &Payload::default().u32(0x40).u32(0x10).u16(1).u16(0xaa64),
        12,
    );
    let strings = Payload::default()
        .u32(0x10)
        .u16(1)
        .u16(2)
        .name(b"a")
        .name(b"bc");
    add(cases, &[0x1019], &strings, 8);
    add(
        cases,
        &[0x113a],
        &Payload::default().u32(0x20).u16(335).u8(1).u8(0),
        8,
    );
    add(cases, &[0x1166], &Payload::default().u16(335), 2);
    add(
        cases,
        &[0x115e],
        &Payload::default().u32(0x10).u16(1).u16(5).u32(0x1003),
        12,
    );
    add(
        cases,
        &[0x112c],
        &Payload::default()
            .u16(0)
            .u16(5)
            .u32(0x10)
            .u32(0x20)
            .u16(1)
            .u16(1),
        16,
    );
}

/// Variables of all storage kinds.
fn variables(cases: &mut Vec<Case>) {
    add(
        cases,
        &[0x113e],
        &Payload::default().u32(0x74).u16(1).name(b"x"),
        6,
    );
    let relative = Payload::default().u32(8).u32(0x74).u16(335).name(b"r");
    add(cases, &[0x1111, 0x1179], &relative, 10);
    let relative_indirect = Payload::default()
        .u32(8)
        .u32(0x74)
        .u32(4)
        .u16(335)
        .name(b"r");
    add(cases, &[0x1171, 0x117b], &relative_indirect, 14);
    let frame = Payload::default().u32(0xffff_fff8).u32(0x74).name(b"f");
    add(cases, &[0x110b, 0x1178], &frame, 8);
    let frame_indirect = Payload::default()
        .u32(0xffff_fff8)
        .u32(0x74)
        .u32(4)
        .name(b"f");
    add(cases, &[0x1170, 0x117a], &frame_indirect, 12);
    add(
        cases,
        &[0x1106],
        &Payload::default().u32(0x74).u16(17).name(b"g"),
        6,
    );
    let attributes = |payload: Payload| payload.u32(0x10).u16(1).u16(1);
    add(
        cases,
        &[0x110a],
        &Payload::default()
            .u32(0x74)
            .u8(2)
            .bytes(&[17, 18])
            .name(b"m"),
        7,
    );
    add(
        cases,
        &[0x1117],
        &Payload::default()
            .u32(0x74)
            .u16(2)
            .u16(17)
            .u16(18)
            .name(b"m"),
        10,
    );
    add(
        cases,
        &[0x1121],
        &attributes(Payload::default().u32(0x74))
            .u8(2)
            .bytes(&[17, 18])
            .name(b"m"),
        15,
    );
    add(
        cases,
        &[0x1123, 0x1131],
        &attributes(Payload::default().u32(0x74))
            .u16(2)
            .u16(17)
            .u16(18)
            .name(b"m"),
        18,
    );
    add(
        cases,
        &[0x111e, 0x112e],
        &attributes(Payload::default().u32(8).u32(0x74)).name(b"a"),
        16,
    );
    add(
        cases,
        &[0x111f, 0x112f],
        &attributes(Payload::default().u32(0x74)).u16(17).name(b"a"),
        14,
    );
    add(
        cases,
        &[0x1122, 0x1130],
        &attributes(Payload::default().u32(8).u32(0x74).u16(335)).name(b"a"),
        18,
    );
    add(
        cases,
        &[0x111a, 0x111b],
        &Payload::default().u32(2).u32(0x1100_0001).name(b"s"),
        8,
    );
    add(
        cases,
        &[0x1120],
        &attributes(Payload::default().u32(2).u32(0x1100_0001)).name(b"s"),
        16,
    );
    add(
        cases,
        &[0x1153],
        &Payload::default().u32(0x74).u32(0x20).u16(0).name(b"fs"),
        10,
    );
    add(
        cases,
        &[0x1176],
        &Payload::default().u32(0x10).u32(0x20).u16(3).u16(1),
        12,
    );
    add(
        cases,
        &[0x1154],
        &Payload::default().u32(0x74).u16(0).u16(1).u16(8).name(b"g"),
        10,
    );
}

/// Def-ranges of all kinds.
fn def_ranges(cases: &mut Vec<Case>) {
    let gap = [4, 0, 2, 0];
    add(
        cases,
        &[0x113f],
        &range(Payload::default().u32(7)).bytes(&gap),
        12,
    );
    add(
        cases,
        &[0x1140],
        &range(Payload::default().u32(7).u32(4)).bytes(&gap),
        16,
    );
    add(
        cases,
        &[0x1142],
        &range(Payload::default().u32(0xffff_fff0)).bytes(&gap),
        12,
    );
    add(cases, &[0x1144], &Payload::default().u32(0xffff_fff0), 4);
    add(
        cases,
        &[0x1141],
        &range(Payload::default().u16(328).u16(0)).bytes(&gap),
        12,
    );
    add(
        cases,
        &[0x1145],
        &range(Payload::default().u16(335).u16(0x41).u32(8)).bytes(&gap),
        16,
    );
    add(
        cases,
        &[0x1177],
        &range(Payload::default().u16(335).u16(0).u32(8).u32(4)).bytes(&gap),
        20,
    );
    add(
        cases,
        &[0x1143],
        &range(Payload::default().u16(328).u16(0).u32(4)).bytes(&gap),
        16,
    );
    let hlsl = range(Payload::default().u16(1).u16(1).u16(0).u16(4))
        .bytes(&gap)
        .u32(0x40);
    add(cases, &[0x1150, 0x1157], &hlsl, 20);
    add(cases, &[0x117f], &Payload::default().u32(1).u32(2), 0);
    add(cases, &[0x1180], &Payload::default().u32(0x10).u16(3), 6);
}

/// Data, constants, types and references.
fn data_and_references(cases: &mut Vec<Case>) {
    let data = Payload::default().u32(0x74).u32(0x10).u16(3).name(b"d");
    add(
        cases,
        &[0x110c, 0x110d, 0x1112, 0x1113, 0x111c, 0x111d],
        &data,
        10,
    );
    add(
        cases,
        &[0x1108, 0x1109],
        &Payload::default().u32(0x1004).name(b"T"),
        4,
    );
    add(cases, &[0x1028], &Payload::default().u32(0x1004), 4);
    let hlsl = Payload::default()
        .u32(0x74)
        .u16(1)
        .u16(2)
        .u16(3)
        .u16(4)
        .u16(5)
        .u16(6)
        .name(b"h");
    add(cases, &[0x1151, 0x1152], &hlsl, 16);
    let hlsl32 = Payload::default()
        .u32(0x74)
        .u32(2)
        .u32(3)
        .u32(4)
        .u32(5)
        .u32(6)
        .u16(1)
        .name(b"h");
    add(cases, &[0x1162, 0x1163], &hlsl32, 26);
    let hlsl_ex = Payload::default()
        .u32(0x74)
        .u32(2)
        .u32(3)
        .u32(4)
        .u32(5)
        .u16(1)
        .name(b"h");
    add(cases, &[0x1164, 0x1165], &hlsl_ex, 22);
    add(cases, &[0x1158], &Payload::default().u32(1).u32(0x20), 0);
    add(
        cases,
        &[0x1107],
        &Payload::default().u32(0x74).u16(0x8004).u32(7).name(b"c"),
        10,
    );
    add(
        cases,
        &[0x112d],
        &Payload::default()
            .u32(0x1100_0002)
            .u16(0x8010)
            .u16(2)
            .bytes(b"hi")
            .name(b"c"),
        10,
    );
    add(
        cases,
        &[0x110e],
        &Payload::default().u32(2).u32(0x10).u16(1).name(b"p"),
        10,
    );
    let reference = Payload::default().u32(0).u32(0x40).u16(1).name(b"r");
    add(
        cases,
        &[0x1125, 0x1127, 0x1129, 0x1126, 0x1128],
        &reference,
        10,
    );
    add(
        cases,
        &[0x100c],
        &Payload::default().u32(0x1001).u32(0x1002).u32(0x10).u16(2),
        14,
    );
}

/// Module, compile, linker and filler records.
fn module_records(cases: &mut Vec<Case>) {
    add(
        cases,
        &[0x1101],
        &Payload::default().u32(0).name(b"a.obj"),
        4,
    );
    let versions = |payload: Payload, count: usize| {
        (0..count).fold(payload, |payload, value| {
            payload.u16(u16::try_from(value).unwrap())
        })
    };
    add(
        cases,
        &[0x1116],
        &versions(Payload::default().u32(1).u16(0xd0), 6)
            .name(b"v")
            .name(b"k")
            .name(b"x")
            .u8(0),
        18,
    );
    add(
        cases,
        &[0x113c],
        &versions(Payload::default().u32(1).u16(0xd0), 8).name(b"v"),
        22,
    );
    add(cases, &[0x114c, 0x117e], &Payload::default().u32(0x1003), 4);
    add(
        cases,
        &[0x113d],
        &Payload::default().u8(0).name(b"cwd").name(b"c:").u8(0),
        1,
    );
    add(cases, &[0x1124], &Payload::default().name(b"std"), 0);
    add(
        cases,
        &[0x1167],
        &Payload::default().u32(3).u16(2).name(b"f"),
        6,
    );
    add(
        cases,
        &[0x1160],
        &Payload::default().u32(3).u16(1).u16(2).name(b"f"),
        8,
    );
    add(
        cases,
        &[0x1136],
        &Payload::default()
            .u16(1)
            .u8(12)
            .u8(0)
            .u32(0x1000)
            .u32(0x200)
            .u32(0x6000_0020)
            .name(b".text"),
        16,
    );
    add(
        cases,
        &[0x1137],
        &Payload::default()
            .u32(0x100)
            .u32(0x6000_0020)
            .u32(0)
            .u16(1)
            .name(b".text$mn"),
        14,
    );
    add(
        cases,
        &[0x1138],
        &Payload::default().u16(3).u16(0x12).name(b"Export"),
        4,
    );
    add(
        cases,
        &[0x113b],
        &Payload::default().u32(1).u32(2).u32(3).bytes(&[2, 0, 6, 0]),
        12,
    );
    add(
        cases,
        &[0x115f],
        &Payload::default().u32(0x10).u16(5).u16(0),
        8,
    );
    add(
        cases,
        &[0x1161],
        &Payload::default().bytes(&[b'a', 0, 0, 0, b'b', 0, 0, 0]),
        0,
    );
    let oem = Payload::default().zeros(16).u32(0).bytes(&[1, 2, 3, 4]);
    add(cases, &[0x0404], &oem, 20);
    add(cases, &[0x0007], &Payload::default().zeros(8), 0);
}
