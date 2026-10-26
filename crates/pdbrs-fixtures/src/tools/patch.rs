//! The two changes to the copy of the `DIA2Dump` sample.
//!
//! `msdia140.dll` is not registered as a COM server, so `CoCreateInstance` fails with
//! `0x80040154` (class not registered). `NoRegCoCreate` from `diacreate.h` loads the DLL
//! directly. It needs `diaguids.lib`, which the sample project does not link.

use std::path::Path;

/// The start of the `CoCreateInstance` call in `DIA2Dump.cpp`.
const CO_CREATE: &str = "CoCreateInstance(__uuidof(DiaSource)";

/// The include line in `stdafx.h` after which the link directive goes.
const DIA2_INCLUDE: &str = "#include \"dia2.h\"";

/// The DLL that `NoRegCoCreate` loads.
pub const DLL: &str = "msdia140.dll";

/// Replace the `CoCreateInstance(__uuidof(DiaSource), ...)` call with `NoRegCoCreate`.
///
/// The call must occur once and have five arguments: the class, the outer object, the
/// context, the interface and the output pointer. `NoRegCoCreate` takes the DLL name, the
/// class, the interface and the output pointer.
pub fn no_reg_co_create(source: &str) -> Result<String, String> {
    let start = find_once(source, CO_CREATE, "DIA2Dump.cpp")?;
    let open = start + "CoCreateInstance".len();
    let close = matching_parenthesis(source, open)
        .ok_or("the CoCreateInstance call in DIA2Dump.cpp has no closing parenthesis")?;
    let arguments = split_arguments(&source[open + 1..close]);
    let [class, _, _, interface, output] = arguments.as_slice() else {
        return Err(format!(
            "the CoCreateInstance call in DIA2Dump.cpp has {} arguments, not 5",
            arguments.len()
        ));
    };
    Ok(format!(
        "{}NoRegCoCreate(L\"{DLL}\", {class}, {interface}, {output}){}",
        &source[..start],
        &source[close + 1..]
    ))
}

/// Include `diacreate.h` and link `diaguids` after the `dia2.h` include line of `stdafx.h`.
///
/// The `#pragma comment(lib, ...)` directive uses the absolute path with `/` separators. The
/// new lines use the line ending of the header.
pub fn link_diaguids(header: &str, diaguids: &Path) -> Result<String, String> {
    let library = diaguids.to_string_lossy().replace('\\', "/");
    if library.contains('"') {
        return Err(format!("the path {library} contains a quote"));
    }
    let start = find_once(header, DIA2_INCLUDE, "stdafx.h")?;
    let line_ending = if header.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let (before, after) = match header[start..].find('\n') {
        Some(offset) => header.split_at(start + offset + 1),
        None => (header, ""),
    };
    let separator = if before.ends_with('\n') {
        ""
    } else {
        line_ending
    };
    Ok(format!(
        "{before}{separator}#include \"diacreate.h\"{line_ending}\
         #pragma comment(lib, \"{library}\"){line_ending}{after}"
    ))
}

/// The byte offset of the only occurrence of `pattern` in `text`.
fn find_once(text: &str, pattern: &str, file: &str) -> Result<usize, String> {
    let mut matches = text.match_indices(pattern).map(|(offset, _)| offset);
    match (matches.next(), matches.next()) {
        (Some(offset), None) => Ok(offset),
        (None, _) => Err(format!("{file} does not contain `{pattern}`")),
        (Some(_), Some(_)) => Err(format!("{file} contains `{pattern}` more than once")),
    }
}

/// The offset of the parenthesis that closes the one at `open`.
fn matching_parenthesis(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0_usize;
    for (offset, byte) in text.bytes().enumerate().skip(open) {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split call arguments at the commas outside parentheses. Collapse whitespace in each.
fn split_arguments(text: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0;
    for (offset, byte) in text.bytes().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                arguments.push(&text[start..offset]);
                start = offset + 1;
            }
            _ => {}
        }
    }
    arguments.push(&text[start..]);
    arguments
        .into_iter()
        .map(|argument| argument.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{link_diaguids, no_reg_co_create};
    use std::path::Path;

    const SAMPLE_CALL: &str = "  // Obtain access to the provider\r\n\r\n  \
        hr = CoCreateInstance(__uuidof(DiaSource),\r\n                        NULL,\r\n    \
        CLSCTX_INPROC_SERVER,\r\n    __uuidof(IDiaDataSource),\r\n    \
        (void **) ppSource);\r\n\r\n  if (FAILED(hr)) {\r\n";

    #[test]
    fn replaces_the_call_and_keeps_the_text_around_it() {
        let patched = no_reg_co_create(SAMPLE_CALL).unwrap();
        assert_eq!(
            patched,
            "  // Obtain access to the provider\r\n\r\n  hr = NoRegCoCreate(L\"msdia140.dll\", \
             __uuidof(DiaSource), __uuidof(IDiaDataSource), (void **) ppSource);\r\n\r\n  \
             if (FAILED(hr)) {\r\n"
        );
    }

    #[test]
    fn rejects_a_missing_repeated_or_changed_call() {
        assert_eq!(
            no_reg_co_create("int main();"),
            Err("DIA2Dump.cpp does not contain `CoCreateInstance(__uuidof(DiaSource)`".to_owned())
        );
        let twice = format!("{SAMPLE_CALL}{SAMPLE_CALL}");
        assert!(
            no_reg_co_create(&twice)
                .unwrap_err()
                .contains("more than once")
        );
        assert_eq!(
            no_reg_co_create("CoCreateInstance(__uuidof(DiaSource), NULL, ppSource);"),
            Err("the CoCreateInstance call in DIA2Dump.cpp has 3 arguments, not 5".to_owned())
        );
        assert!(no_reg_co_create("CoCreateInstance(__uuidof(DiaSource), (").is_err());
    }

    #[test]
    fn links_diaguids_after_the_dia2_include() {
        let header = "#include <tchar.h>\r\n#include \"dia2.h\"\r\n\r\n";
        let diaguids = Path::new(r"C:\VS\DIA SDK\lib\amd64\diaguids.lib");
        assert_eq!(
            link_diaguids(header, diaguids).unwrap(),
            "#include <tchar.h>\r\n#include \"dia2.h\"\r\n#include \"diacreate.h\"\r\n\
             #pragma comment(lib, \"C:/VS/DIA SDK/lib/amd64/diaguids.lib\")\r\n\r\n"
        );
    }

    #[test]
    fn links_diaguids_after_a_last_line_without_ending() {
        assert_eq!(
            link_diaguids("#include \"dia2.h\"", Path::new("/sdk/diaguids.lib")).unwrap(),
            "#include \"dia2.h\"\n#include \"diacreate.h\"\n\
             #pragma comment(lib, \"/sdk/diaguids.lib\")\n"
        );
    }

    #[test]
    fn rejects_a_header_without_the_include() {
        assert_eq!(
            link_diaguids("#pragma once\n", Path::new("x.lib")),
            Err("stdafx.h does not contain `#include \"dia2.h\"`".to_owned())
        );
    }
}
