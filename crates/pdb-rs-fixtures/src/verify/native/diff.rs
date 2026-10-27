//! Compare two tool outputs line by line and classify the differences.
//!
//! Some differences are explained and do not make a check fail:
//!
//! - [`Class::Container`]: the container layout lines `Number of streams` and `Block Size`. A
//!   semantic rebuild drops nil streams and streams that nothing names.
//! - [`Class::PublicOrder`]: lines of public symbols that both outputs have, at other positions.
//!   LLD puts some 8-byte names in another hash bucket than the bucket of the name. The rebuild
//!   uses the bucket of the name, so the order of the public symbols changes.
//! - [`Exits::SameFailure`]: both runs of a tool fail with the same exit code.
//!
//! All other differences are unexplained.

use std::collections::HashMap;

/// The maximum edit distance of the line difference. A larger distance gives a difference
/// without the order of the lines: each line that one output has more often than the other.
const MAX_DISTANCE: usize = 2000;

/// The side of a changed line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Side {
    /// The original output has the line, the rebuilt output does not.
    Removed,
    /// The rebuilt output has the line, the original output does not.
    Added,
}

/// One line that only one output has at this position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Change<'a> {
    /// The side of the line.
    pub side: Side,
    /// The index (0-based) of the line in its output.
    pub line: usize,
    /// The text of the line.
    pub text: &'a str,
}

/// Return the changed lines of two texts, in order.
///
/// The function finds a shortest edit script with the Myers algorithm. If the edit distance
/// is more than [`MAX_DISTANCE`], it returns the lines that one text has more often than the
/// other, without order.
pub(crate) fn line_diff<'a>(left: &'a str, right: &'a str) -> Vec<Change<'a>> {
    let left: Vec<&str> = left.lines().collect();
    let right: Vec<&str> = right.lines().collect();
    let prefix = left
        .iter()
        .zip(&right)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = left[prefix..]
        .iter()
        .rev()
        .zip(right[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let left_middle = &left[prefix..left.len() - suffix];
    let right_middle = &right[prefix..right.len() - suffix];

    let mut ids = HashMap::new();
    let mut intern = |line: &&'a str| {
        let next = ids.len();
        *ids.entry(*line).or_insert(next)
    };
    let a: Vec<usize> = left_middle.iter().map(&mut intern).collect();
    let b: Vec<usize> = right_middle.iter().map(&mut intern).collect();
    let edits = myers(&a, &b).unwrap_or_else(|| unordered(&a, &b));
    edits
        .into_iter()
        .map(|(side, index)| {
            let text = match side {
                Side::Removed => left_middle[index],
                Side::Added => right_middle[index],
            };
            Change {
                side,
                line: prefix + index,
                text,
            }
        })
        .collect()
}

/// Return the edit script from `old` to `new` as `(side, index)` pairs, or `None` if the edit
/// distance is more than [`MAX_DISTANCE`].
fn myers(old: &[usize], new: &[usize]) -> Option<Vec<(Side, usize)>> {
    let (old_len, new_len) = (signed(old.len()), signed(new.len()));
    let limit = (old.len() + new.len()).min(MAX_DISTANCE);
    let offset = signed(limit) + 1;
    let at = |k: isize| usize::try_from(k + offset).unwrap_or(0);
    // `ends[k]` is the furthest `x` on diagonal `k`.
    let mut ends = vec![0_isize; 2 * limit + 3];
    // `trace[d]` holds `ends[k]` for `k` in `-d..=d` after step `d`.
    let mut trace: Vec<Vec<isize>> = Vec::new();
    for d in 0..=signed(limit) {
        let mut k = -d;
        let mut done = false;
        while k <= d {
            let mut x = if k == -d || (k != d && ends[at(k - 1)] < ends[at(k + 1)]) {
                ends[at(k + 1)]
            } else {
                ends[at(k - 1)] + 1
            };
            let mut y = x - k;
            while x < old_len && y < new_len && old[index(x)] == new[index(y)] {
                x += 1;
                y += 1;
            }
            ends[at(k)] = x;
            done |= x >= old_len && y >= new_len;
            k += 2;
        }
        trace.push(ends[at(-d)..=at(d)].to_vec());
        if done {
            return Some(backtrack(&trace, old_len, new_len));
        }
    }
    None
}

/// Follow the trace of [`myers`] back from the ends of both texts and return the edits in
/// order.
fn backtrack(trace: &[Vec<isize>], old_len: isize, new_len: isize) -> Vec<(Side, usize)> {
    let mut edits = Vec::new();
    let (mut x, mut y) = (old_len, new_len);
    for d in (1..trace.len()).rev() {
        let d_signed = signed(d);
        let previous = &trace[d - 1];
        let get = |k: isize| previous[index(k + d_signed - 1)];
        let k = x - y;
        let previous_k = if k == -d_signed || (k != d_signed && get(k - 1) < get(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let previous_x = get(previous_k);
        let previous_y = previous_x - previous_k;
        while x > previous_x && y > previous_y {
            x -= 1;
            y -= 1;
        }
        if x == previous_x {
            edits.push((Side::Added, index(previous_y)));
        } else {
            edits.push((Side::Removed, index(previous_x)));
        }
        x = previous_x;
        y = previous_y;
    }
    edits.reverse();
    edits
}

/// Return the lines that one side has more often than the other, without order: first the
/// removed lines, then the added lines.
fn unordered(a: &[usize], b: &[usize]) -> Vec<(Side, usize)> {
    let mut balance: HashMap<usize, isize> = HashMap::new();
    for &line in a {
        *balance.entry(line).or_default() += 1;
    }
    for &line in b {
        *balance.entry(line).or_default() -= 1;
    }
    let mut edits = Vec::new();
    for (side, lines, sign) in [(Side::Removed, a, 1), (Side::Added, b, -1)] {
        for (position, line) in lines.iter().enumerate() {
            if let Some(count) = balance.get_mut(line).filter(|count| **count * sign > 0) {
                *count -= sign;
                edits.push((side, position));
            }
        }
    }
    edits
}

/// Convert a length to `isize`. Line counts always fit.
fn signed(value: usize) -> isize {
    isize::try_from(value).unwrap_or(isize::MAX)
}

/// Convert a non-negative position to `usize`.
fn index(value: isize) -> usize {
    usize::try_from(value).unwrap_or(0)
}

/// The tool output that a comparison reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dump {
    /// A normalized `llvm-pdbutil dump --all` output. The public symbols are the lines of the
    /// `Public Symbols` section.
    Pdbutil,
    /// A masked `DIA2Dump -all` output. The public symbols are the `PublicSymbol:` lines.
    Dia,
}

/// The class of a changed line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    /// A container layout line.
    Container,
    /// A public symbol line that both outputs have, at another position.
    PublicOrder,
    /// Any other change.
    Unexplained,
}

/// The classified differences of two outputs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Comparison {
    /// The number of changed lines in the container layout class.
    pub container: usize,
    /// The number of changed lines in the public order class.
    pub public_order: usize,
    /// The unexplained changed lines: `-<line>: <text>` for a removed line and `+<line>:
    /// <text>` for an added line, with 1-based line numbers.
    pub unexplained: Vec<String>,
}

impl Comparison {
    /// Compare an original and a rebuilt output.
    pub(crate) fn new(dump: Dump, original: &str, rebuilt: &str) -> Self {
        let changes = line_diff(original, rebuilt);
        let sections = [original, rebuilt].map(|text| public_lines(dump, text));
        let mut comparison = Self::default();
        for (change, class) in changes.iter().zip(classify(&changes, |change| {
            let side = side_index(change.side);
            sections[side].get(change.line).copied().unwrap_or(false)
        })) {
            match class {
                Class::Container => comparison.container += 1,
                Class::PublicOrder => comparison.public_order += 1,
                Class::Unexplained => comparison.unexplained.push(format!(
                    "{}{}: {}",
                    if change.side == Side::Removed {
                        '-'
                    } else {
                        '+'
                    },
                    change.line + 1,
                    change.text
                )),
            }
        }
        comparison
    }

    /// The number of changed lines.
    pub(crate) fn changed(&self) -> usize {
        self.container + self.public_order + self.unexplained.len()
    }

    /// Describe the comparison, for example `12 lines differ (explained: 2 container layout,
    /// 10 LLD public bucket order)`.
    pub(crate) fn describe(&self) -> String {
        if self.changed() == 0 {
            return "same".to_owned();
        }
        let mut explained = Vec::new();
        if self.container > 0 {
            explained.push(format!("{} container layout", self.container));
        }
        if self.public_order > 0 {
            explained.push(format!("{} LLD public bucket order", self.public_order));
        }
        let mut parts = Vec::new();
        if !self.unexplained.is_empty() {
            parts.push(format!("{} UNEXPLAINED", self.unexplained.len()));
        }
        if !explained.is_empty() {
            parts.push(format!("explained: {}", explained.join(", ")));
        }
        format!("{} lines differ ({})", self.changed(), parts.join("; "))
    }
}

/// Return for each line of an output if it shows a public symbol.
fn public_lines(dump: Dump, text: &str) -> Vec<bool> {
    let lines: Vec<&str> = text.lines().collect();
    match dump {
        Dump::Dia => lines
            .iter()
            .map(|line| line.starts_with("PublicSymbol:"))
            .collect(),
        Dump::Pdbutil => {
            // A section starts with its name on the line before a line of `=` characters.
            let mut public = false;
            (0..lines.len())
                .map(|position| {
                    if lines
                        .get(position + 1)
                        .is_some_and(|next| next.starts_with("====="))
                    {
                        public = lines[position].trim() == "Public Symbols";
                    }
                    public
                })
                .collect()
        }
    }
}

/// Classify each change.
///
/// A change is in the container class if its text starts with `Number of streams:` or
/// `Block Size:`. A change for which `public` is `true` is in the public order class if the
/// other side has a public change with the same text. Each change pairs with one change of the
/// other side only.
pub(crate) fn classify(changes: &[Change<'_>], public: impl Fn(&Change<'_>) -> bool) -> Vec<Class> {
    let mut counts: HashMap<&str, [usize; 2]> = HashMap::new();
    for change in changes.iter().filter(|change| public(change)) {
        counts.entry(change.text).or_default()[side_index(change.side)] += 1;
    }
    let mut used: HashMap<(Side, &str), usize> = HashMap::new();
    changes
        .iter()
        .map(|change| {
            let trimmed = change.text.trim_start();
            if trimmed.starts_with("Number of streams:") || trimmed.starts_with("Block Size:") {
                return Class::Container;
            }
            if !public(change) {
                return Class::Unexplained;
            }
            let pairs = counts
                .get(change.text)
                .map_or(0, |[removed, added]| *removed.min(added));
            let used = used.entry((change.side, change.text)).or_default();
            if *used < pairs {
                *used += 1;
                Class::PublicOrder
            } else {
                Class::Unexplained
            }
        })
        .collect()
}

/// The index of a side in a pair of counts.
fn side_index(side: Side) -> usize {
    usize::from(side == Side::Added)
}

/// The exit codes of the two runs of a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exits {
    /// Both runs exit with code 0.
    Success,
    /// Both runs fail with the same exit code. This is explained.
    SameFailure(Option<i32>),
    /// The runs exit with different codes. This is unexplained.
    Different(Option<i32>, Option<i32>),
}

impl Exits {
    /// Classify the exit codes of the original and the rebuilt run.
    pub(crate) fn new(original: Option<i32>, rebuilt: Option<i32>) -> Self {
        if original != rebuilt {
            Self::Different(original, rebuilt)
        } else if original == Some(0) {
            Self::Success
        } else {
            Self::SameFailure(original)
        }
    }

    /// Return `true` unless the exit codes differ.
    pub(crate) fn explained(self) -> bool {
        !matches!(self, Self::Different(..))
    }

    /// Describe the exit codes, or return an empty text for two successful runs.
    pub(crate) fn describe(self) -> String {
        let code =
            |code: Option<i32>| code.map_or_else(|| "none".to_owned(), |code| code.to_string());
        match self {
            Self::Success => String::new(),
            Self::SameFailure(exit) => format!(", explained: both fail with exit {}", code(exit)),
            Self::Different(original, rebuilt) => format!(
                ", UNEXPLAINED: exit {} for the original, {} for the rebuild",
                code(original),
                code(rebuilt)
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Change, Class, Comparison, Dump, Exits, Side, classify, line_diff, myers};

    fn changes(left: &str, right: &str) -> Vec<(Side, usize, String)> {
        line_diff(left, right)
            .into_iter()
            .map(|change| (change.side, change.line, change.text.to_owned()))
            .collect()
    }

    #[test]
    fn finds_a_shortest_line_difference() {
        assert_eq!(changes("a\nb\nc\n", "a\nb\nc\n"), Vec::new());
        assert_eq!(
            changes("a\nb\nc\nd\n", "a\nx\nc\nd\ne\n"),
            vec![
                (Side::Removed, 1, "b".to_owned()),
                (Side::Added, 1, "x".to_owned()),
                (Side::Added, 4, "e".to_owned()),
            ]
        );
        assert_eq!(changes("", "a\n"), vec![(Side::Added, 0, "a".to_owned())]);
        assert_eq!(changes("a\n", ""), vec![(Side::Removed, 0, "a".to_owned())]);
        // A moved line gives one removed and one added line.
        assert_eq!(changes("p\nq\nr\n", "q\nr\np\n").len(), 2);
        let a: Vec<usize> = (0..50).collect();
        let b: Vec<usize> = (0..50).rev().collect();
        assert_eq!(myers(&a, &b).unwrap().len(), 98);
    }

    #[test]
    fn classifies_container_and_public_order_lines() {
        let original = "\
  Block Size: 512
  Number of streams: 74
                     Public Symbols
============================================================
     12 | S_PUB32 [size = 20] `g_c_name`
           flags = none, addr = 0003:0000
     40 | S_PUB32 [size = 20] `main`
           flags = function, addr = 0001:0010
";
        let rebuilt = "\
  Block Size: 4096
  Number of streams: 70
                     Public Symbols
============================================================
     40 | S_PUB32 [size = 20] `main`
           flags = function, addr = 0001:0010
     12 | S_PUB32 [size = 20] `g_c_name`
           flags = none, addr = 0003:0000
";
        let comparison = Comparison::new(Dump::Pdbutil, original, rebuilt);
        assert_eq!(comparison.container, 4);
        assert_eq!(comparison.public_order, 4);
        assert_eq!(comparison.unexplained, Vec::<String>::new());
        assert_eq!(
            comparison.describe(),
            "8 lines differ (explained: 4 container layout, 4 LLD public bucket order)"
        );
    }

    #[test]
    fn other_changes_are_unexplained() {
        // The same reorder outside the public symbols is unexplained.
        let comparison = Comparison::new(Dump::Dia, "a\nb\nc\n", "b\na\nc\n");
        assert_eq!(comparison.public_order, 0);
        assert_eq!(comparison.unexplained, vec!["-1: a", "+2: a"]);
        assert_eq!(comparison.describe(), "2 lines differ (2 UNEXPLAINED)");

        // A changed public symbol has no partner on the other side.
        let original = "PublicSymbol: [00001000][0001:00000000] f\nPublicSymbol: [00002000][0001:00001000] g\n";
        let rebuilt = "PublicSymbol: [00002000][0001:00001000] g\nPublicSymbol: [00001000][0001:00000000] f\nPublicSymbol: [00003000][0001:00002000] h\n";
        let comparison = Comparison::new(Dump::Dia, original, rebuilt);
        assert_eq!(comparison.public_order, 2);
        assert_eq!(
            comparison.unexplained,
            vec!["+3: PublicSymbol: [00003000][0001:00002000] h"]
        );
        assert_eq!(Comparison::new(Dump::Dia, "x\n", "x\n").describe(), "same");
    }

    #[test]
    fn pairs_each_public_change_once() {
        let change = |side, text| Change {
            side,
            line: 0,
            text,
        };
        let list = [
            change(Side::Removed, "PublicSymbol: f"),
            change(Side::Removed, "PublicSymbol: f"),
            change(Side::Added, "PublicSymbol: f"),
        ];
        assert_eq!(
            classify(&list, |_| true),
            vec![Class::PublicOrder, Class::Unexplained, Class::PublicOrder]
        );
    }

    #[test]
    fn classifies_exit_codes() {
        assert_eq!(Exits::new(Some(0), Some(0)), Exits::Success);
        assert_eq!(Exits::new(Some(1), Some(1)), Exits::SameFailure(Some(1)));
        assert!(Exits::new(Some(1), Some(1)).explained());
        assert!(!Exits::new(Some(0), Some(1)).explained());
        assert_eq!(Exits::new(Some(0), Some(0)).describe(), "");
        assert_eq!(
            Exits::new(Some(1), Some(1)).describe(),
            ", explained: both fail with exit 1"
        );
        assert_eq!(
            Exits::new(Some(0), None).describe(),
            ", UNEXPLAINED: exit 0 for the original, none for the rebuild"
        );
    }
}
