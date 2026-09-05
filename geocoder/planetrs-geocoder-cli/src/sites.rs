//! Reading sites and writing their answers, in one CSV dialect for both.
//!
//! Semicolon-separated, UTF-8, a header naming the five columns. A field in double quotes may hold
//! semicolons, doubled quotes and line breaks. Every field is echoed exactly as it was written and
//! trimmed only for the lookup, so an answer row can be matched back to its site by eye.

use planetrs_geocoder::{Answer, Site};
use thiserror::Error;

/// The columns a site file has, in the order answers are written.
pub const COLUMNS: [&str; 5] = ["label", "address", "postal code", "city", "country"];

/// The first line of every answer, from a file or from one site.
pub const ANSWER_HEADER: &str = "label;address;postal code;city;country;lat;lon;precision";

/// The precision written when nothing, not even a commune, was found.
pub const NOT_FOUND: &str = "NotFound";

const DELIMITER: char = ';';
const BYTE_ORDER_MARK: char = '\u{feff}';
const BAD_ROWS_SHOWN: usize = 20;

/// One site as written: its five fields in [`COLUMNS`] order, and the line it starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The line of the file the row starts on, counted from one; zero for a site given alone.
    pub line: usize,
    /// The fields exactly as written, quotes and spaces resolved by the CSV rules only.
    pub fields: [String; 5],
}

impl Row {
    /// The site the geocoder reads: every field trimmed.
    pub fn site(&self) -> Site {
        let [label, address, postcode, town, country] =
            self.fields.clone().map(|field| field.trim().to_string());
        Site {
            label,
            address,
            postcode,
            town,
            country,
        }
    }
}

/// A whole site file, read and checked before anything is geocoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteFile {
    /// Every site, in the order of the file.
    pub rows: Vec<Row>,
    /// Whether the file opened with a byte order mark, which the answer file then carries too.
    pub had_byte_order_mark: bool,
}

/// Why sites could not be read. Each is a fault of the input, never of the tool.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SitesError {
    /// The bytes are not UTF-8.
    #[error("line {line} is not UTF-8: save the file as \"CSV UTF-8\" and run again")]
    NotUtf8 {
        /// The first line holding a byte that is not UTF-8.
        line: usize,
    },
    /// Nothing at all, not even a header.
    #[error("the file is empty: its first line must be the header {}", COLUMNS.join(";"))]
    Empty,
    /// A header separated by commas, as a spreadsheet saves by default in some languages.
    #[error(
        "the header is separated by commas, and this tool reads semicolons: save the file as CSV \
         with ';' as the separator, header {}",
        COLUMNS.join(";")
    )]
    CommaSeparated,
    /// A header that does not name the five columns.
    #[error("the header must name the five columns {}, in any order; it names {found}", COLUMNS.join(";"))]
    Header {
        /// The header as written.
        found: String,
    },
    /// Rows without five fields, every one of them listed.
    #[error("{}", describe_bad_rows(.rows))]
    FieldCount {
        /// The line each bad row starts on, and how many fields it has.
        rows: Vec<(usize, usize)>,
    },
    /// A site given on the command line without five fields.
    #[error(
        "a site is five fields, \"{}\", and this one has {count}: quote it as one argument, or see \
         planetrs-geocoder --help for the commands",
        COLUMNS.join(";")
    )]
    OneSite {
        /// How many fields it has.
        count: usize,
    },
}

fn describe_bad_rows(rows: &[(usize, usize)]) -> String {
    let mut text = format!(
        "{} row{} without the five fields {}, nothing was geocoded:",
        rows.len(),
        if rows.len() == 1 { "" } else { "s" },
        COLUMNS.join(";")
    );
    for (line, count) in rows.iter().take(BAD_ROWS_SHOWN) {
        text.push_str(&format!(
            "\n  line {line}: {count} field{}",
            if *count == 1 { "" } else { "s" }
        ));
    }
    if rows.len() > BAD_ROWS_SHOWN {
        text.push_str(&format!("\n  and {} more", rows.len() - BAD_ROWS_SHOWN));
    }
    text
}

/// Splits CSV text into records, each with the line it starts on.
///
/// A quote opens or closes quoting wherever it stands, and a doubled quote inside quoting is one
/// quote, so a field like `"""Le Moulin"""` reads `"Le Moulin"`. A line break inside quoting belongs
/// to the field; outside it ends the record, with or without a carriage return before it.
pub fn records(text: &str) -> Vec<(usize, Vec<String>)> {
    let mut records = Vec::new();
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut line = 1;
    let mut record_line = 1;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '"' if quoted && characters.peek() == Some(&'"') => {
                field.push('"');
                characters.next();
            }
            '"' => quoted = !quoted,
            '\r' if !quoted && characters.peek() == Some(&'\n') => {}
            '\n' if !quoted => {
                fields.push(std::mem::take(&mut field));
                records.push((record_line, std::mem::take(&mut fields)));
                line += 1;
                record_line = line;
            }
            '\n' => {
                field.push('\n');
                line += 1;
            }
            other if other == DELIMITER && !quoted => fields.push(std::mem::take(&mut field)),
            other => field.push(other),
        }
    }
    if !field.is_empty() || !fields.is_empty() {
        fields.push(field);
        records.push((record_line, fields));
    }
    records
}

fn is_blank(fields: &[String]) -> bool {
    fields.iter().all(|field| field.trim().is_empty())
}

/// Reads a site file: the header, then every row, refusing the whole file at the first fault of
/// its format and listing every row that does not have five fields.
pub fn read_site_file(bytes: &[u8]) -> Result<SiteFile, SitesError> {
    let text = std::str::from_utf8(bytes).map_err(|error| SitesError::NotUtf8 {
        line: 1 + bytes[..error.valid_up_to()]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count(),
    })?;
    let (had_byte_order_mark, text) = match text.strip_prefix(BYTE_ORDER_MARK) {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let mut all = records(text).into_iter().filter(|(_, fields)| !is_blank(fields));
    let (_, header) = all.next().ok_or(SitesError::Empty)?;
    let order = column_order(&header)?;
    let mut rows = Vec::new();
    let mut bad = Vec::new();
    for (line, fields) in all {
        if fields.len() == COLUMNS.len() {
            rows.push(Row {
                line,
                fields: order.map(|at| fields[at].clone()),
            });
        } else {
            bad.push((line, fields.len()));
        }
    }
    if !bad.is_empty() {
        return Err(SitesError::FieldCount { rows: bad });
    }
    Ok(SiteFile {
        rows,
        had_byte_order_mark,
    })
}

/// Where each of [`COLUMNS`] stands in the header.
fn column_order(header: &[String]) -> Result<[usize; 5], SitesError> {
    let written = header.join(";");
    if header.len() == 1 && header[0].contains(',') {
        return Err(SitesError::CommaSeparated);
    }
    let names: Vec<String> = header.iter().map(|name| name.trim().to_lowercase()).collect();
    let position = |column: &str| names.iter().position(|name| name == column);
    let found = COLUMNS.map(position);
    match found {
        [
            Some(label),
            Some(address),
            Some(postcode),
            Some(city),
            Some(country),
        ] if names.len() == COLUMNS.len() => Ok([label, address, postcode, city, country]),
        _ => Err(SitesError::Header { found: written }),
    }
}

/// Reads one site given as a single argument, `label;address;postal code;city;country`.
pub fn read_one_site(text: &str) -> Result<Row, SitesError> {
    let fields: Vec<String> = records(text).into_iter().flat_map(|(_, fields)| fields).collect();
    let count = fields.len();
    let fields: [String; 5] = fields.try_into().map_err(|_| SitesError::OneSite { count })?;
    Ok(Row { line: 0, fields })
}

/// A field as CSV writes it: quoted when it holds a separator, a quote or a line break.
pub fn quoted(field: &str) -> String {
    if field.contains([DELIMITER, '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// The answer line for a row: its five fields as written, then latitude, longitude and precision.
pub fn answer_line(row: &Row, answer: Option<&Answer>) -> String {
    let mut fields: Vec<String> = row.fields.iter().map(|field| quoted(field)).collect();
    match answer {
        Some(answer) => {
            fields.push(format!("{:.6}", answer.latitude()));
            fields.push(format!("{:.6}", answer.longitude()));
            fields.push(answer.precision.to_string());
        }
        None => fields.extend([String::new(), String::new(), NOT_FOUND.to_string()]),
    }
    fields.join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "label;address;postal code;city;country";

    fn file(text: &str) -> Result<SiteFile, SitesError> {
        read_site_file(text.as_bytes())
    }

    fn row(fields: [&str; 5]) -> Row {
        Row {
            line: 2,
            fields: fields.map(String::from),
        }
    }

    #[test]
    fn given_a_quoted_field_when_split_then_its_quotes_belong_to_the_format_not_the_value() {
        let fields = &records(r#"00000;ACME;"""Le Moulin""";99999"#)[0].1;

        assert_eq!(fields[2], "\"Le Moulin\"");
    }

    #[test]
    fn given_a_quoted_field_holding_the_separator_when_split_then_it_is_not_split_there() {
        assert_eq!(records(r#"a;"b;c";d"#)[0].1, vec!["a", "b;c", "d"]);
    }

    #[test]
    fn given_an_empty_trailing_field_when_split_then_it_is_kept() {
        assert_eq!(records("a;b;")[0].1, vec!["a", "b", ""]);
    }

    #[test]
    fn given_a_quoted_line_break_when_split_then_the_record_goes_on_and_the_next_line_is_counted() {
        let read = records("a;\"b\nc\";d\ne;f");

        assert_eq!(
            read[0],
            (1, vec!["a".to_string(), "b\nc".to_string(), "d".to_string()])
        );
        assert_eq!(read[1].0, 3);
    }

    #[test]
    fn given_windows_line_endings_when_split_then_no_field_keeps_the_carriage_return() {
        assert_eq!(records("a;b\r\nc;d\r\n")[1].1, vec!["c", "d"]);
    }

    #[test]
    fn given_a_file_with_a_byte_order_mark_when_read_then_the_mark_is_dropped_and_remembered() {
        let read = file(&format!(
            "\u{feff}{HEADER}\nAcme;8 Route des Tilleuls;89100;Sens;France\n"
        ))
        .unwrap();

        assert!(read.had_byte_order_mark);
        assert_eq!(read.rows[0].fields[0], "Acme");
    }

    #[test]
    fn given_a_header_in_another_order_and_case_when_read_then_the_fields_come_out_in_column_order() {
        let read =
            file("Country; City ;Postal Code;Address;LABEL\nFrance;Sens;89100;8 Route des Tilleuls;Acme\n")
                .unwrap();

        assert_eq!(
            read.rows[0].fields,
            ["Acme", "8 Route des Tilleuls", "89100", "Sens", "France"].map(String::from)
        );
    }

    #[test]
    fn given_blank_lines_when_read_then_they_are_skipped_and_line_numbers_still_count_them() {
        let read = file(&format!(
            "{HEADER}\n\n;;;;\nAcme;8 Route des Tilleuls;89100;Sens;France\n"
        ))
        .unwrap();

        assert_eq!(read.rows.len(), 1);
        assert_eq!(read.rows[0].line, 4);
    }

    #[test]
    fn given_a_comma_separated_header_when_read_then_the_separator_is_named() {
        assert_eq!(
            file("label,address,postal code,city,country\n"),
            Err(SitesError::CommaSeparated)
        );
    }

    #[test]
    fn given_a_header_missing_a_column_when_read_then_the_header_found_is_named() {
        assert_eq!(
            file("label;address;city;country\n"),
            Err(SitesError::Header {
                found: "label;address;city;country".into()
            })
        );
    }

    #[test]
    fn given_a_header_with_a_sixth_column_when_read_then_it_is_refused() {
        assert!(matches!(
            file(&format!("{HEADER};id\n")),
            Err(SitesError::Header { .. })
        ));
    }

    #[test]
    fn given_rows_without_five_fields_when_read_then_every_one_is_listed_and_nothing_is_kept() {
        let read = file(&format!(
            "{HEADER}\nAcme;8 Route des Tilleuls;89100;Sens;France;extra\nAcme;8 Route des Tilleuls;89100;Sens;France\nAcme;Sens\n"
        ));

        assert_eq!(
            read,
            Err(SitesError::FieldCount {
                rows: vec![(2, 6), (4, 2)]
            })
        );
    }

    #[test]
    fn given_bytes_that_are_not_utf8_when_read_then_their_line_is_named() {
        let mut bytes = format!("{HEADER}\nAcme;").into_bytes();
        bytes.extend([0xE9, b'\n']);

        assert_eq!(read_site_file(&bytes), Err(SitesError::NotUtf8 { line: 2 }));
    }

    #[test]
    fn given_an_empty_file_when_read_then_it_asks_for_the_header() {
        assert_eq!(file(""), Err(SitesError::Empty));
    }

    #[test]
    fn given_fields_with_spaces_around_them_when_a_site_is_made_then_they_are_trimmed_for_the_lookup() {
        let site = row([" Acme ", " 8 Route des Tilleuls", "89100 ", "Sens", " France"]).site();

        assert_eq!(
            (site.label.as_str(), site.address.as_str(), site.postcode.as_str()),
            ("Acme", "8 Route des Tilleuls", "89100")
        );
    }

    #[test]
    fn given_one_site_of_five_fields_when_read_then_it_is_a_row() {
        assert_eq!(
            read_one_site("Acme;8 Route des Tilleuls;89100;Sens;France")
                .unwrap()
                .fields[3],
            "Sens"
        );
    }

    #[test]
    fn given_one_site_of_four_fields_when_read_then_the_count_is_named() {
        assert_eq!(
            read_one_site("8 Route des Tilleuls;89100;Sens;France"),
            Err(SitesError::OneSite { count: 4 })
        );
    }

    #[test]
    fn given_no_answer_when_a_line_is_written_then_it_ends_with_two_empty_coordinates_and_not_found() {
        let line = answer_line(
            &row(["Acme", "8 Route des Tilleuls", "89100", "Sens", "France"]),
            None,
        );

        assert_eq!(line, "Acme;8 Route des Tilleuls;89100;Sens;France;;;NotFound");
    }

    #[test]
    fn given_a_field_holding_quotes_and_a_separator_when_written_then_it_reads_back_the_same() {
        let written = row(["\"Le Moulin\"", "5;7 Rue Haute", "89100", "Sens", "France"]);

        let line = answer_line(&written, None);

        assert_eq!(records(&line)[0].1[..5], written.fields);
    }
}
