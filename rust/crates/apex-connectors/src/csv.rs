//! CSV records without the quoted-field blind spot: a placement or campaign
//! name containing a comma (or a newline) must not shift every later column,
//! and a doubled quote must not end the field early. Both report clients fold
//! these records; a row whose shape the parser cannot honor still fails
//! closed in the caller as a typed schema error.

/// Parse CSV text into trimmed records: RFC 4180 quoting (embedded commas,
/// embedded newlines, `""` escapes), CRLF tolerated, blank lines skipped.
/// An unterminated quote consumes the rest of the input as that field; the
/// caller's column-count/typed-parse checks then refuse the feed.
#[must_use]
pub fn records(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if in_quotes {
            match character {
                '"' => {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        field.push('"');
                    } else {
                        in_quotes = false;
                    }
                }
                _ => field.push(character),
            }
            continue;
        }
        match character {
            // A quote opens a quoted field only in field position: after
            // commas/newlines or leading whitespace, never inside a value.
            '"' if field.chars().all(char::is_whitespace) => {
                field.clear();
                in_quotes = true;
            }
            ',' => {
                row.push(std::mem::take(&mut field).trim().to_string());
            }
            '\n' => {
                row.push(std::mem::take(&mut field).trim().to_string());
                if row.iter().any(|value| !value.is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            // CR outside a quoted field only ever precedes LF.
            '\r' => {}
            _ => field.push(character),
        }
    }
    row.push(field.trim().to_string());
    if row.iter().any(|value| !value.is_empty()) {
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::records;

    #[test]
    fn quoted_commas_and_newlines_keep_their_columns() {
        let text = "name,placement,revenue\n\
                    unit,\"foo, bar\",1.5\n\
                    unit,\"two\nlines\",2.5\n";
        let rows = records(text);
        assert_eq!(
            rows,
            vec![
                vec!["name", "placement", "revenue"],
                vec!["unit", "foo, bar", "1.5"],
                vec!["unit", "two\nlines", "2.5"],
            ]
        );
    }

    #[test]
    fn doubled_quotes_and_crlf_round_trip() {
        let rows = records("a,b\r\n\"say \"\"hi\"\"\",2\r\n");
        assert_eq!(rows, vec![vec!["a", "b"], vec!["say \"hi\"", "2"]]);
    }

    #[test]
    fn blank_lines_are_skipped_and_quotes_after_spaces_open() {
        let rows = records("\n h1, h2 \n\n  \"x,y\",z\n");
        assert_eq!(rows, vec![vec!["h1", "h2"], vec!["x,y", "z"]]);
    }

    #[test]
    fn a_quote_inside_an_unquoted_value_stays_literal() {
        let rows = records("note,1\n5\" pipe,2\n");
        assert_eq!(rows, vec![vec!["note", "1"], vec!["5\" pipe", "2"]]);
    }
}
