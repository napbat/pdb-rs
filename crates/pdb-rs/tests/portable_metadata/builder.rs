//! A builder of synthetic Portable PDB metadata for the tests.
//!
//! The builder writes the metadata root, the stream headers and each stream at a 4-byte
//! aligned offset. A test can change any field, add raw streams or change the stream headers
//! before it assembles the bytes.

/// One stream of the metadata root.
#[derive(Clone, Debug)]
pub struct Stream {
    /// The name without the terminator.
    pub name: Vec<u8>,
    /// The stream bytes. The builder pads them to a multiple of 4.
    pub bytes: Vec<u8>,
    /// An offset that replaces the computed offset in the stream header.
    pub offset: Option<u32>,
    /// A size that replaces the padded size in the stream header.
    pub size: Option<u32>,
}

impl Stream {
    /// Make a stream with the computed offset and size.
    pub fn new(name: &str, bytes: Vec<u8>) -> Self {
        Self {
            name: name.as_bytes().to_vec(),
            bytes,
            offset: None,
            size: None,
        }
    }
}

/// The fields of a synthetic Portable PDB.
#[derive(Clone, Debug)]
pub struct Builder {
    /// The metadata root version (1.1 is valid).
    pub root_version: (u16, u16),
    /// The version string without the terminator.
    pub version: Vec<u8>,
    /// The 20-byte PDB ID.
    pub id: [u8; 20],
    /// The entry point token.
    pub entry_point: u32,
    /// The external tables and their row counts, in table number order.
    pub external: Vec<(u8, u32)>,
    /// The `#~` version (2.0 is valid).
    pub tables_version: (u8, u8),
    /// The `HeapSizes` flags.
    pub heap_sizes: u8,
    /// The `Sorted` mask.
    pub sorted: u64,
    /// The tables of `#~`: table number, row count and the row bytes, in table number order.
    pub tables: Vec<(u8, u32, Vec<u8>)>,
    /// The `#Strings` heap, or `None` to omit the stream.
    pub strings: Option<Vec<u8>>,
    /// The `#US` heap, or `None` to omit the stream.
    pub user_strings: Option<Vec<u8>>,
    /// The `#GUID` heap, or `None` to omit the stream.
    pub guids: Option<Vec<u8>>,
    /// The `#Blob` heap, or `None` to omit the stream.
    pub blobs: Option<Vec<u8>>,
}

impl Default for Builder {
    fn default() -> Self {
        let mut id = [0; 20];
        for (index, byte) in id.iter_mut().enumerate() {
            *byte = u8::try_from(index).unwrap() + 1;
        }
        Self {
            root_version: (1, 1),
            version: b"PDB v1.0".to_vec(),
            id,
            entry_point: 0x0600_0001,
            external: vec![(0x00, 1), (0x06, 3)],
            tables_version: (2, 0),
            heap_sizes: 0,
            sorted: 0,
            tables: Vec::new(),
            strings: Some(b"\0name\0".to_vec()),
            user_strings: Some(vec![0]),
            guids: Some((1..=32).collect()),
            blobs: Some(vec![0, 3, b'a', b'b', b'c']),
        }
    }
}

impl Builder {
    /// Add a table with `rows` rows of `bytes`.
    pub fn table(mut self, table: u8, rows: u32, bytes: Vec<u8>) -> Self {
        self.tables.push((table, rows, bytes));
        self.tables.sort_by_key(|(number, _, _)| *number);
        self
    }

    /// The `#Pdb` stream bytes.
    pub fn pdb_stream(&self) -> Vec<u8> {
        let mut bytes = self.id.to_vec();
        bytes.extend_from_slice(&self.entry_point.to_le_bytes());
        let mask = self
            .external
            .iter()
            .fold(0_u64, |mask, (table, _)| mask | 1 << table);
        bytes.extend_from_slice(&mask.to_le_bytes());
        for (_, rows) in &self.external {
            bytes.extend_from_slice(&rows.to_le_bytes());
        }
        bytes
    }

    /// The `#~` stream bytes.
    pub fn table_stream(&self) -> Vec<u8> {
        let mut bytes = vec![0, 0, 0, 0, self.tables_version.0, self.tables_version.1];
        bytes.push(self.heap_sizes);
        bytes.push(1);
        let valid = self
            .tables
            .iter()
            .fold(0_u64, |mask, (table, _, _)| mask | 1 << table);
        bytes.extend_from_slice(&valid.to_le_bytes());
        bytes.extend_from_slice(&self.sorted.to_le_bytes());
        for (_, rows, _) in &self.tables {
            bytes.extend_from_slice(&rows.to_le_bytes());
        }
        for (_, _, rows) in &self.tables {
            bytes.extend_from_slice(rows);
        }
        bytes
    }

    /// The streams in the usual Roslyn order.
    pub fn streams(&self) -> Vec<Stream> {
        let mut streams = vec![
            Stream::new("#Pdb", self.pdb_stream()),
            Stream::new("#~", self.table_stream()),
        ];
        for (name, heap) in [
            ("#Strings", &self.strings),
            ("#US", &self.user_strings),
            ("#GUID", &self.guids),
            ("#Blob", &self.blobs),
        ] {
            if let Some(bytes) = heap {
                streams.push(Stream::new(name, bytes.clone()));
            }
        }
        streams
    }

    /// Assemble the default streams.
    pub fn build(&self) -> Vec<u8> {
        self.assemble(&self.streams())
    }

    /// Assemble the metadata root with `streams`.
    pub fn assemble(&self, streams: &[Stream]) -> Vec<u8> {
        let mut version = self.version.clone();
        version.push(0);
        while !version.len().is_multiple_of(4) {
            version.push(0);
        }
        let mut header = b"BSJB".to_vec();
        header.extend_from_slice(&self.root_version.0.to_le_bytes());
        header.extend_from_slice(&self.root_version.1.to_le_bytes());
        header.extend_from_slice(&0_u32.to_le_bytes());
        header.extend_from_slice(&u32::try_from(version.len()).unwrap().to_le_bytes());
        header.extend_from_slice(&version);
        header.extend_from_slice(&0_u16.to_le_bytes());
        header.extend_from_slice(&u16::try_from(streams.len()).unwrap().to_le_bytes());
        let names_len: usize = streams
            .iter()
            .map(|stream| 8 + (stream.name.len() + 4) / 4 * 4)
            .sum();

        let mut offset = header.len() + names_len;
        let mut body = Vec::new();
        for stream in streams {
            let mut bytes = stream.bytes.clone();
            while !bytes.len().is_multiple_of(4) {
                bytes.push(0);
            }
            let computed = u32::try_from(offset).unwrap();
            header.extend_from_slice(&stream.offset.unwrap_or(computed).to_le_bytes());
            let size = u32::try_from(bytes.len()).unwrap();
            header.extend_from_slice(&stream.size.unwrap_or(size).to_le_bytes());
            let mut name = stream.name.clone();
            name.push(0);
            while !name.len().is_multiple_of(4) {
                name.push(0);
            }
            header.extend_from_slice(&name);
            offset += bytes.len();
            body.extend_from_slice(&bytes);
        }
        header.extend_from_slice(&body);
        header
    }
}

/// Encode a little-endian value of `width` bytes (2 or 4).
pub fn value(width: u8, value: u32) -> Vec<u8> {
    if width == 2 {
        u16::try_from(value).unwrap().to_le_bytes().to_vec()
    } else {
        value.to_le_bytes().to_vec()
    }
}

/// Concatenate encoded columns.
pub fn row(columns: &[Vec<u8>]) -> Vec<u8> {
    columns.concat()
}
