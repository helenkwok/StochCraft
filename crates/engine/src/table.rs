//! Reading a table of delimited text: CSV as RFC 4180 describes it, as any simulation tool,
//! spreadsheet or script writes it.
//!
//! - Cells may be quoted with double quotes; inside quotes a delimiter or a line break belongs
//!   to the cell, and `""` is a quote. Surrounding spaces are trimmed.
//! - Lines end with LF or CRLF; a byte order mark at the start is skipped; blank lines are left
//!   out.
//! - The delimiter is a comma, a semicolon or a tab: of those found in the first record, the one
//!   that splits the most of the first records into as many cells as the first (a tab before a
//!   semicolon before a comma on a tie). A comma where none of them is found.
//! - With a semicolon delimiter a number may have a decimal comma (`3,25`), as spreadsheets in
//!   many regions write it.
//! - At most [`MAX_CELLS`] cells, so a very large file is refused with a clear message rather
//!   than taking all the memory.

/// The most cells a table may have.
pub const MAX_CELLS: u64 = 5_000_000;

/// How many records the delimiter is chosen from.
const SNIFF_RECORDS: usize = 20;

/// What separates the cells of a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delimiter {
    Comma,
    Semicolon,
    Tab,
}

impl Delimiter {
    fn char(self) -> char {
        match self {
            Delimiter::Comma => ',',
            Delimiter::Semicolon => ';',
            Delimiter::Tab => '\t',
        }
    }

    /// Its name in answers.
    pub fn name(self) -> &'static str {
        match self {
            Delimiter::Comma => "comma",
            Delimiter::Semicolon => "semicolon",
            Delimiter::Tab => "tab",
        }
    }
}

/// One record: its cells, and the line of the text it starts on (from 1), for messages.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub line: u64,
    pub cells: Vec<String>,
}

/// A table read from delimited text: its records in order, the first usually a header.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub delimiter: Delimiter,
    pub records: Vec<Record>,
}

impl Table {
    /// Whether numbers may have a decimal comma: with a semicolon delimiter.
    pub fn decimal_comma(&self) -> bool {
        self.delimiter == Delimiter::Semicolon
    }

    /// The number a cell holds, finite or not (`inf` and `NaN` read as numbers, for the caller
    /// to refuse); `None` for an empty cell or one that is not a number. With a semicolon
    /// delimiter a decimal comma is read as a point.
    pub fn number(&self, cell: &str) -> Option<f64> {
        let cell = cell.trim();
        if cell.is_empty() {
            return None;
        }
        if self.decimal_comma() && cell.contains(',') && !cell.contains('.') {
            cell.replacen(',', ".", 1).parse().ok()
        } else {
            cell.parse().ok()
        }
    }
}

/// Reads `text` as a table (see the module's rules). An error, with the line, for a quote that
/// is never closed, or for more than [`MAX_CELLS`] cells.
pub fn read(text: &str) -> Result<Table, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let delimiter = sniff(text)?;
    let records = parse(text, delimiter.char(), None)?;
    Ok(Table { delimiter, records })
}

/// The delimiter of `text`, from its first records.
fn sniff(text: &str) -> Result<Delimiter, String> {
    let mut best: Option<(usize, Delimiter)> = None;
    for d in [Delimiter::Tab, Delimiter::Semicolon, Delimiter::Comma] {
        let records = parse(text, d.char(), Some(SNIFF_RECORDS))?;
        let Some(first) = records.first() else {
            return Ok(Delimiter::Comma);
        };
        let width = first.cells.len();
        if width < 2 {
            continue;
        }
        let same = records.iter().filter(|r| r.cells.len() == width).count();
        if best.is_none_or(|(n, _)| same > n) {
            best = Some((same, d));
        }
    }
    Ok(best.map_or(Delimiter::Comma, |(_, d)| d))
}

/// The records being built by [`parse`].
struct Builder {
    records: Vec<Record>,
    cells: Vec<String>,
    cell: String,
    /// Cells read so far, against [`MAX_CELLS`].
    count: u64,
    /// Whether a cell of the record being read was quoted (so it is not a blank line).
    any_quoted: bool,
    /// The line the record being read starts on.
    start: u64,
}

impl Builder {
    /// Ends the cell being read.
    fn end_cell(&mut self) -> Result<(), String> {
        self.count += 1;
        if self.count > MAX_CELLS {
            return Err(format!(
                "the table has more than {MAX_CELLS} cells: split it into smaller files, or leave out columns not needed"
            ));
        }
        self.cells
            .push(std::mem::take(&mut self.cell).trim().to_owned());
        Ok(())
    }

    /// Ends the record being read; a blank line (one empty cell, not quoted) is left out.
    fn end_record(&mut self) -> Result<(), String> {
        self.end_cell()?;
        let blank = !self.any_quoted
            && self.cells.len() == 1
            && self.cells.first().is_some_and(String::is_empty);
        if blank {
            self.count -= 1;
            self.cells.clear();
        } else {
            self.records.push(Record {
                line: self.start,
                cells: std::mem::take(&mut self.cells),
            });
        }
        self.any_quoted = false;
        Ok(())
    }
}

/// The records of `text` split at `delimiter` (at most `limit` of them), blank ones left out.
fn parse(text: &str, delimiter: char, limit: Option<usize>) -> Result<Vec<Record>, String> {
    let mut b = Builder {
        records: Vec::new(),
        cells: Vec::new(),
        cell: String::new(),
        count: 0,
        any_quoted: false,
        start: 1,
    };
    let mut line = 1u64;
    // Inside quotes; and whether the cell being read was quoted (a second quote after its
    // closing one is kept as text).
    let (mut quoted, mut was_quoted) = (false, false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    b.cell.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                '\n' => {
                    b.cell.push('\n');
                    line += 1;
                }
                c => b.cell.push(c),
            }
            continue;
        }
        match c {
            '"' if !was_quoted && b.cell.trim().is_empty() => {
                b.cell.clear();
                quoted = true;
                was_quoted = true;
                b.any_quoted = true;
            }
            c if c == delimiter => {
                b.end_cell()?;
                was_quoted = false;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                b.end_record()?;
                was_quoted = false;
                if limit.is_some_and(|l| b.records.len() >= l) {
                    return Ok(b.records);
                }
                line += 1;
                b.start = line;
            }
            c => b.cell.push(c),
        }
    }
    if quoted {
        return Err(format!("line {}: a double quote is not closed", b.start));
    }
    if !(b.cell.is_empty() && b.cells.is_empty() && !b.any_quoted) {
        b.end_record()?;
    }
    Ok(b.records)
}

#[cfg(test)]
#[allow(clippy::panic)] // A wrong answer fails the test.
mod tests {
    use super::*;

    fn cells(t: &Table) -> Vec<Vec<&str>> {
        t.records
            .iter()
            .map(|r| r.cells.iter().map(String::as_str).collect())
            .collect()
    }

    #[test]
    fn quotes_line_breaks_and_the_byte_order_mark() {
        let t = read(
            "\u{feff}id,\"service, min\",note\r\n1,2.5,\"say \"\"hi\"\"\"\n\n2,3,\"two\nlines\"\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(t.delimiter, Delimiter::Comma);
        assert_eq!(
            cells(&t),
            vec![
                vec!["id", "service, min", "note"],
                vec!["1", "2.5", "say \"hi\""],
                vec!["2", "3", "two\nlines"],
            ]
        );
        // Each record's line is the one it starts on.
        let lines: Vec<u64> = t.records.iter().map(|r| r.line).collect();
        assert_eq!(lines, vec![1, 2, 4]);
        assert!(
            read("a\n\"1\n2\n").is_err_and(|e| e.contains("line 2") && e.contains("not closed"))
        );
        assert!(read("").is_ok_and(|t| t.records.is_empty()));
        // No line break at the end, and an empty last cell.
        let t = read("a,b\n1,").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(cells(&t), vec![vec!["a", "b"], vec!["1", ""]]);
    }

    #[test]
    fn the_delimiter_is_found_from_the_first_records() {
        let d = |text: &str| read(text).map(|t| t.delimiter);
        assert_eq!(d("a;b\n1;2\n"), Ok(Delimiter::Semicolon));
        assert_eq!(d("a\tb\n1\t2\n"), Ok(Delimiter::Tab));
        assert_eq!(d("a,b\n1,2\n"), Ok(Delimiter::Comma));
        assert_eq!(d("1.5\n2\n"), Ok(Delimiter::Comma));
        // A semicolon file with decimal commas: both split every record evenly, and the
        // semicolon wins the tie.
        assert_eq!(d("1,5;2,5\n3,5;4,5\n"), Ok(Delimiter::Semicolon));
        // A comma file whose header has a semicolon in a name: only commas split evenly.
        assert_eq!(d("a,b;c\n1,2\n3,4\n"), Ok(Delimiter::Comma));
        // A delimiter inside quotes does not count.
        assert_eq!(d("\"a;b\",c\n1,2\n"), Ok(Delimiter::Comma));
    }

    #[test]
    fn a_decimal_comma_only_with_semicolons() {
        let semi = read("a;b\n1,5;2\n").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(semi.number("1,5"), Some(1.5));
        assert_eq!(semi.number(" 2 "), Some(2.0));
        assert_eq!(semi.number(""), None);
        assert_eq!(semi.number("x"), None);
        let comma = read("a,b\n1,2\n").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(comma.number("1,5"), None);
        assert_eq!(comma.number("1.5e1"), Some(15.0));
        assert!(comma.number("inf").is_some_and(f64::is_infinite));
    }

    #[test]
    fn too_many_cells_are_refused() {
        // MAX_CELLS + 1 cells in one record, without building a huge table first.
        let text = ",".repeat(MAX_CELLS as usize);
        assert!(read(&text).is_err_and(|e| e.contains("more than 5000000 cells")));
    }
}
