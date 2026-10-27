//! The address queries of the clean `DIA2Dump` comparison: `-l`, `-sym` and `-fpo`.
//!
//! `DIA2Dump` stops at the first query without an answer (for example `-fpo` at an address
//! without frame data) and exits with -1. The check then counts that query as one without an
//! answer and runs the queries after it again.

use std::iter;
use std::path::Path;

use super::super::super::super::native::consumers::dia2dump;
use super::Context;

/// The number of queries in one `DIA2Dump` run.
const QUERY_BATCH: usize = 500;

/// The exit code of `DIA2Dump` after a query without an answer: `wmain` returns -1.
const STOPPED: i32 = -1;

/// How `DIA2Dump` separates the answers of one query kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Split {
    /// An empty line ends each answer (`-l`, `-fpo`).
    EmptyLine,
    /// A `Displacement = ` line starts each answer (`-sym`).
    Displacement,
}

/// Split the output of a query run into the complete answers and the lines after them, or
/// return `None` for an output that does not have the form.
fn split_answers<'a>(
    lines: impl Iterator<Item = &'a str>,
    split: Split,
) -> Option<(Vec<Vec<String>>, Vec<String>)> {
    let mut answers = Vec::new();
    let mut answer: Vec<String> = Vec::new();
    match split {
        Split::EmptyLine => {
            for line in lines {
                if line.is_empty() {
                    answers.push(std::mem::take(&mut answer));
                } else {
                    answer.push(line.to_owned());
                }
            }
            Some((answers, answer))
        }
        Split::Displacement => {
            let mut started = false;
            for line in lines {
                if line.starts_with("Displacement = ") {
                    if started {
                        answers.push(std::mem::take(&mut answer));
                    }
                    started = true;
                } else if !started && !line.is_empty() {
                    return None;
                }
                if started {
                    answer.push(line.to_owned());
                }
            }
            if started {
                answers.push(answer);
            }
            Some((answers, Vec::new()))
        }
    }
}

/// Return the answers of the query `option <rva> extra...` for each RVA of `rvas` on `pdb`
/// (`None` for a query without an answer).
///
/// One `DIA2Dump` run takes up to [`QUERY_BATCH`] queries. A run that stops at a query
/// without an answer (exit code [`STOPPED`]) gives the complete answers of the queries
/// before it. The next run starts after that query.
pub(super) fn query(
    context: &Context<'_>,
    query: (&str, &[&str], Split),
    rvas: &[u32],
    pdb: &Path,
) -> Result<Vec<Option<Vec<String>>>, String> {
    let (option, extra, split) = query;
    let mut answers = Vec::with_capacity(rvas.len());
    while answers.len() < rvas.len() {
        let start = answers.len();
        let batch = &rvas[start..rvas.len().min(start + QUERY_BATCH)];
        let arguments: Vec<String> = batch
            .iter()
            .flat_map(|rva| {
                iter::once(option.to_owned())
                    .chain(iter::once(format!("0{rva:X}")))
                    .chain(extra.iter().map(|&argument| argument.to_owned()))
            })
            .collect();
        let run = dia2dump(context.dia2dump, &arguments, pdb)?;
        match split_answers(run.text.lines(), split) {
            Some((parts, rest))
                if run.success() && rest.is_empty() && parts.len() == batch.len() =>
            {
                answers.extend(parts.into_iter().map(Some));
            }
            // The query after the complete answers has no answer. A stopped `-fpo` query
            // prints an error line.
            Some((parts, _)) if run.code == Some(STOPPED) && parts.len() < batch.len() => {
                answers.extend(parts.into_iter().map(Some));
                answers.push(None);
            }
            _ => {
                return Err(format!(
                    "DIA2Dump {option} with {} queries on {} gives an output without the answer form (exit {:?}): {}",
                    batch.len(),
                    pdb.display(),
                    run.code,
                    crate::step::last_lines(&run.text, 3)
                ));
            }
        }
    }
    Ok(answers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|&line| line.to_owned()).collect()
    }

    #[test]
    fn answers_split_at_their_separators() {
        let text = "\tline 1 at [00001000][0001:00000000]\n\n\tline 2\n\tline 3\n\n";
        let (answers, rest) = split_answers(text.lines(), Split::EmptyLine).unwrap();
        assert_eq!(
            answers,
            [
                lines(&["\tline 1 at [00001000][0001:00000000]"]),
                lines(&["\tline 2", "\tline 3"])
            ]
        );
        assert_eq!(rest.len(), 0);
        // A stopped `-fpo` query prints an error line after the complete answers.
        let text = "\tframe 1\n\nERROR - DumpFPO() frameByRVA invalid RVA: 0x00001000\n";
        let (answers, rest) = split_answers(text.lines(), Split::EmptyLine).unwrap();
        assert_eq!(answers, [lines(&["\tframe 1"])]);
        assert_eq!(rest.len(), 1);
        let text = "Displacement = 0x1\nFunction\n\nParent\nDisplacement = 0x0\nData\n";
        let (answers, rest) = split_answers(text.lines(), Split::Displacement).unwrap();
        assert_eq!(answers.len(), 2);
        assert_eq!(answers[0].len(), 4);
        assert_eq!(rest.len(), 0);
        assert_eq!(
            split_answers("Function\n".lines(), Split::Displacement),
            None
        );
    }
}
