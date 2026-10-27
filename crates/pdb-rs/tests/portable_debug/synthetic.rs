//! A synthetic Portable PDB with rows in all eight debug tables.
//!
//! [`Synthetic::valid`] passes all validation rules. A test changes one column value and
//! calls [`Synthetic::bytes`]. All tables are small, so each index is 2 bytes wide.

use pdb_rs::portable::compressed::encode_unsigned;
use pdb_rs::portable::debug::{HashAlgorithm, Language};

use crate::builder::{Builder, row, value};

/// The external tables of the `#Pdb` stream: `Module`, `TypeRef`, `TypeDef`, `MethodDef`,
/// `StandAloneSig`, `TypeSpec` and `AssemblyRef`.
pub const EXTERNAL: [(u8, u32); 7] = [
    (0x00, 1),
    (0x01, 5),
    (0x02, 3),
    (0x06, 4),
    (0x11, 2),
    (0x1b, 1),
    (0x23, 2),
];

/// The `#Strings` offsets of the names `a`, `b` and `c`.
pub const NAME_A: u32 = 1;
/// The `#Strings` offset of `b`.
pub const NAME_B: u32 = 3;
/// The `#Strings` offset of `c`.
pub const NAME_C: u32 = 5;

/// The column values of each debug table, in column order.
#[derive(Clone, Debug)]
pub struct Synthetic {
    /// The `#Blob` heap.
    pub blobs: Vec<u8>,
    /// `Document`: name, hash algorithm, hash, language.
    pub documents: Vec<[u32; 4]>,
    /// `MethodDebugInformation`: document, sequence points.
    pub methods: Vec<[u32; 2]>,
    /// `LocalScope`: method, import scope, variable list, constant list, start, length.
    pub scopes: Vec<[u32; 6]>,
    /// `LocalVariable`: attributes, index, name.
    pub variables: Vec<[u32; 3]>,
    /// `LocalConstant`: name, signature.
    pub constants: Vec<[u32; 2]>,
    /// `ImportScope`: parent, imports.
    pub import_scopes: Vec<[u32; 2]>,
    /// `StateMachineMethod`: `MoveNext` method, kickoff method.
    pub state_machines: Vec<[u32; 2]>,
    /// `CustomDebugInformation`: coded parent, kind GUID index, value.
    pub custom: Vec<[u32; 3]>,
}

impl Synthetic {
    /// Make a PDB that passes all validation rules.
    ///
    /// - Two documents, `/src/a.cs` and `/src/b.cs`.
    /// - Four methods: method 1 has points in document 1, method 2 has points in both
    ///   documents, methods 3 and 4 have no points.
    /// - Method 1 has a root scope with two nested scopes; method 2 has one scope.
    /// - Two import scopes; scope 2 has scope 1 as its parent.
    pub fn valid() -> Self {
        let mut pdb = Self {
            blobs: vec![0],
            documents: Vec::new(),
            methods: Vec::new(),
            scopes: Vec::new(),
            variables: Vec::new(),
            constants: Vec::new(),
            import_scopes: Vec::new(),
            state_machines: Vec::new(),
            custom: Vec::new(),
        };
        let src = pdb.blob(b"src");
        let hash = pdb.blob(&[0x5a; 32]);
        for file in [&b"a.cs"[..], b"b.cs"] {
            let part = pdb.blob(file);
            let name = pdb.blob(&[
                b'/',
                0,
                u8::try_from(src).unwrap(),
                u8::try_from(part).unwrap(),
            ]);
            pdb.documents.push([name, 2, hash, 1]);
        }
        // Method 1: (0x0, 10:3-10:8), hidden at 0x2, (0x5, 5:4-6:2).
        let single = pdb.blob(&[
            0x01, 0x00, 0x00, 0x05, 0x0a, 0x03, 0x02, 0x00, 0x00, 0x03, 0x01, 0x7d, 0x77, 0x02,
        ]);
        // Method 2: (0x0, document 1, 1:1-1:2), then (0x4, document 2, 2:1-2:3).
        let multi = pdb.blob(&[
            0x00, 0x01, 0x00, 0x00, 0x01, 0x01, 0x01, 0x00, 0x02, 0x04, 0x00, 0x02, 0x02, 0x00,
        ]);
        pdb.methods = vec![[1, single], [0, multi], [0, 0], [0, 0]];
        pdb.scopes = vec![
            [1, 1, 1, 1, 0x00, 0x20],
            [1, 1, 2, 2, 0x02, 0x0e],
            [1, 2, 3, 2, 0x10, 0x08],
            [2, 2, 3, 2, 0x00, 0x10],
        ];
        pdb.variables = vec![[0, 0, NAME_A], [1, 1, NAME_B], [0, 0, NAME_A]];
        let constant = pdb.blob(&[0x08, 0x05, 0x00, 0x00, 0x00]);
        pdb.constants = vec![[NAME_C, constant]];
        let system = pdb.blob(b"System");
        let alias = pdb.blob(b"Alias");
        let root = pdb.blob(&[0x01, u8::try_from(system).unwrap()]);
        // Kind 9 (alias type): alias, then TypeRef 2 (2 << 2 | 1).
        let child = pdb.blob(&[0x09, u8::try_from(alias).unwrap(), 0x09]);
        pdb.import_scopes = vec![[0, root], [1, child]];
        pdb.state_machines = vec![[2, 1]];
        // Parents: MethodDef 1 (tag 0) and MethodDef 2.
        pdb.custom = vec![[1 << 5, 1, 0], [2 << 5, 1, 0]];
        pdb
    }

    /// Add a blob to the `#Blob` heap and return its offset. The empty blob is offset 0.
    pub fn blob(&mut self, bytes: &[u8]) -> u32 {
        if bytes.is_empty() {
            return 0;
        }
        let offset = u32::try_from(self.blobs.len()).unwrap();
        let length = encode_unsigned(u32::try_from(bytes.len()).unwrap()).unwrap();
        self.blobs.extend_from_slice(length.as_bytes());
        self.blobs.extend_from_slice(bytes);
        offset
    }

    /// Assemble the metadata.
    pub fn bytes(&self) -> Vec<u8> {
        let mut guids = Language::CSharp.guid().to_vec();
        guids.extend_from_slice(&HashAlgorithm::Sha256.guid());
        let mut builder = Builder {
            external: EXTERNAL.to_vec(),
            strings: Some(b"\0a\0b\0c\0".to_vec()),
            guids: Some(guids),
            blobs: Some(self.blobs.clone()),
            ..Builder::default()
        };
        let mut add = |table: u8, rows: Vec<Vec<u32>>, widths: &[u8]| {
            let count = u32::try_from(rows.len()).unwrap();
            let bytes = rows
                .iter()
                .flat_map(|columns| {
                    let encoded: Vec<Vec<u8>> = columns
                        .iter()
                        .zip(widths)
                        .map(|(&column, &width)| value(width, column))
                        .collect();
                    row(&encoded)
                })
                .collect();
            if count > 0 {
                builder = builder.clone().table(table, count, bytes);
            }
        };
        add(
            0x30,
            self.documents.iter().map(|row| row.to_vec()).collect(),
            &[2; 4],
        );
        add(
            0x31,
            self.methods.iter().map(|row| row.to_vec()).collect(),
            &[2; 2],
        );
        add(
            0x32,
            self.scopes.iter().map(|row| row.to_vec()).collect(),
            &[2, 2, 2, 2, 4, 4],
        );
        add(
            0x33,
            self.variables.iter().map(|row| row.to_vec()).collect(),
            &[2; 3],
        );
        add(
            0x34,
            self.constants.iter().map(|row| row.to_vec()).collect(),
            &[2; 2],
        );
        add(
            0x35,
            self.import_scopes.iter().map(|row| row.to_vec()).collect(),
            &[2; 2],
        );
        add(
            0x36,
            self.state_machines.iter().map(|row| row.to_vec()).collect(),
            &[2; 2],
        );
        add(
            0x37,
            self.custom.iter().map(|row| row.to_vec()).collect(),
            &[2; 3],
        );
        builder.build()
    }
}
