//! The source path remap of the `remap cases:` sections of `verify-native-edit` and
//! `verify-edit`.
//!
//! The build root is the directory that holds the fixture sources (`fixtures\`) and the build
//! outputs (`target\fixtures\<fixture>\`): the `output_root` of the fixture manifest without
//! `target\fixtures`. [`BuildRoot::remap`] replaces the root with `/_/` if it starts a path and
//! a separator or the end of the path follows. The comparison ignores the ASCII case and treats
//! `/` and `\` as the same byte, because MSVC writes some paths in lowercase and Clang and Rust
//! write some paths with `/`.

use std::fs;
use std::path::Path;

use crate::json::{self, Value};

/// The value that replaces the build root.
const NEW_ROOT: &[u8] = b"/_";

/// The build root of the fixtures.
#[derive(Clone, Debug)]
pub(crate) struct BuildRoot {
    /// The root as the manifest gives it.
    root: Vec<u8>,
}

/// Return `true` if two path bytes are the same without ASCII case and separator kind.
fn same(left: u8, right: u8) -> bool {
    let normal = |byte: u8| {
        if byte == b'/' {
            b'\\'
        } else {
            byte.to_ascii_lowercase()
        }
    };
    normal(left) == normal(right)
}

impl BuildRoot {
    /// Read the build root from the fixture manifest in `fixtures`.
    pub(crate) fn from_manifest(fixtures: &Path) -> Result<Self, String> {
        let path = fixtures.join("manifest.json");
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let manifest = json::parse(&text)?;
        let Some(output) = manifest.get("output_root").and_then(Value::as_str) else {
            return Err(format!("{} has no `output_root`", path.display()));
        };
        let output = output.as_bytes();
        let suffix = br"\target\fixtures";
        let cut = output.len().checked_sub(suffix.len());
        match cut {
            Some(cut)
                if output[cut..]
                    .iter()
                    .zip(suffix)
                    .all(|(&left, &right)| same(left, right)) =>
            {
                Ok(Self {
                    root: output[..cut].to_vec(),
                })
            }
            _ => Err(format!(
                "the `output_root` of {} does not end with target\\fixtures",
                path.display()
            )),
        }
    }

    /// Return the root as text.
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.root).into_owned()
    }

    /// Return `true` if `bytes` starts with the root.
    fn starts(&self, bytes: &[u8]) -> bool {
        bytes.len() >= self.root.len()
            && bytes
                .iter()
                .zip(&self.root)
                .all(|(&left, &right)| same(left, right))
    }

    /// Return the new value of `path`: `/_` and the rest of the path if the root starts the
    /// path and a separator or the end follows. Return `None` for another path.
    pub(crate) fn remap(&self, path: &[u8]) -> Option<Vec<u8>> {
        if !self.starts(path) {
            return None;
        }
        let rest = &path[self.root.len()..];
        if !rest.is_empty() && rest[0] != b'\\' && rest[0] != b'/' {
            return None;
        }
        let mut new = NEW_ROOT.to_vec();
        if !rest.is_empty() {
            new.push(b'/');
            new.extend_from_slice(&rest[1..]);
        }
        Some(new)
    }

    /// Return the offset of each occurrence of the root in `bytes`.
    pub(crate) fn find(&self, bytes: &[u8]) -> Vec<usize> {
        let mut found = Vec::new();
        if self.root.is_empty() {
            return found;
        }
        for start in 0..bytes.len().saturating_sub(self.root.len() - 1) {
            if self.starts(&bytes[start..]) {
                found.push(start);
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::BuildRoot;

    #[test]
    fn remap_replaces_only_a_leading_root() {
        let root = BuildRoot {
            root: br"C:\build\app".to_vec(),
        };
        assert_eq!(
            root.remap(br"c:/BUILD/app\src\main.cpp"),
            Some(br"/_/src\main.cpp".to_vec())
        );
        assert_eq!(root.remap(br"C:\build\app"), Some(b"/_".to_vec()));
        assert_eq!(root.remap(br"C:\build\application\x.cpp"), None);
        assert_eq!(root.remap(br"-I C:\build\app\include"), None);
        assert_eq!(root.find(br"x C:\BUILD\app\y c:/build/app"), vec![2, 17]);
    }
}
