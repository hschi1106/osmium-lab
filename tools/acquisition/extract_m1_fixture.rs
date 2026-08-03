use std::{env, error::Error, fs, io, ops::Range, path::PathBuf};

const EXTRACTOR_VERSION: u16 = 4;
const FIRST_PAGE: u16 = 1;
const LAST_PAGE: u16 = 16;
const EXPECTED_SNAPSHOT_COUNT: usize = 3_597;
const EXPECTED_REALTIME_COUNT: usize = 70_199;
const STOCK_SNAPSHOT_FORMAT_FIELD: &[u8] = br#""format":"STOCK_SNAPSHOT""#;
const STOCK_REALTIME_FORMAT_FIELD: &[u8] = br#""format":"STOCK_REALTIME""#;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os().skip(1);
    let source_root = PathBuf::from(
        arguments
            .next()
            .ok_or("usage: extract_m1_fixture <source-complete-directory> <output-directory>")?,
    );
    let output_path = PathBuf::from(
        arguments
            .next()
            .ok_or("usage: extract_m1_fixture <source-complete-directory> <output-directory>")?,
    );
    if arguments.next().is_some() {
        return Err("extract_m1_fixture accepts exactly two arguments".into());
    }

    let mut page_outputs = Vec::new();
    let mut snapshot_count = 0_usize;
    let mut realtime_count = 0_usize;

    for page_number in FIRST_PAGE..=LAST_PAGE {
        let page_name = format!("{page_number:04}.json");
        let page_path = source_root.join("pages").join(page_name);
        let page = fs::read(&page_path)?;
        let mut page_output = Vec::new();
        let item_ranges = item_ranges(&page).map_err(|message| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {message}", page_path.display()),
            )
        })?;

        for (item_index, range) in item_ranges.into_iter().enumerate() {
            let record = &page[range];
            if contains_bytes(record, STOCK_SNAPSHOT_FORMAT_FIELD) {
                snapshot_count += 1;
            } else if contains_bytes(record, STOCK_REALTIME_FORMAT_FIELD) {
                realtime_count += 1;
            } else {
                continue;
            }
            if record.first() != Some(&b'{') || record.last() != Some(&b'}') {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{} items[{item_index}] is not a JSON object",
                        page_path.display()
                    ),
                )
                .into());
            }

            page_output.extend_from_slice(record);
            page_output.push(b'\n');
        }

        page_outputs.push((format!("{page_number:04}.jsonl"), page_output));
    }

    if snapshot_count != EXPECTED_SNAPSHOT_COUNT || realtime_count != EXPECTED_REALTIME_COUNT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "expected {EXPECTED_SNAPSHOT_COUNT} STOCK_SNAPSHOT and \
                 {EXPECTED_REALTIME_COUNT} STOCK_REALTIME records, found \
                 {snapshot_count} and {realtime_count}"
            ),
        )
        .into());
    }

    fs::create_dir_all(&output_path)?;
    for (file_name, bytes) in &page_outputs {
        fs::write(output_path.join(file_name), bytes)?;
    }
    println!(
        "extractor_version={EXTRACTOR_VERSION} files={} snapshots={snapshot_count} \
         realtime={realtime_count}",
        page_outputs.len()
    );
    Ok(())
}

fn item_ranges(page: &[u8]) -> Result<Vec<Range<usize>>, String> {
    let array_start = find_items_array(page)?;
    let mut ranges = Vec::new();
    let mut cursor = skip_whitespace(page, array_start + 1);

    if page.get(cursor) == Some(&b']') {
        return Ok(ranges);
    }

    loop {
        let start = cursor;
        let end = json_value_end(page, start)?;
        ranges.push(start..end);
        cursor = skip_whitespace(page, end);

        match page.get(cursor) {
            Some(b',') => cursor = skip_whitespace(page, cursor + 1),
            Some(b']') => return Ok(ranges),
            Some(byte) => {
                return Err(format!(
                    "expected ',' or ']' after items value, found byte 0x{byte:02x}"
                ));
            }
            None => return Err("unterminated items array".into()),
        }
    }
}

fn find_items_array(page: &[u8]) -> Result<usize, String> {
    const ITEMS_KEY: &[u8] = b"\"items\"";

    for key_start in 0..page.len().saturating_sub(ITEMS_KEY.len() - 1) {
        if page.get(key_start..key_start + ITEMS_KEY.len()) != Some(ITEMS_KEY) {
            continue;
        }

        let colon = skip_whitespace(page, key_start + ITEMS_KEY.len());
        if page.get(colon) != Some(&b':') {
            continue;
        }

        let array_start = skip_whitespace(page, colon + 1);
        if page.get(array_start) == Some(&b'[') {
            return Ok(array_start);
        }
    }

    Err("top-level items array was not found".into())
}

fn json_value_end(input: &[u8], start: usize) -> Result<usize, String> {
    match input.get(start) {
        Some(b'{') | Some(b'[') => compound_value_end(input, start),
        Some(b'"') => string_end(input, start),
        Some(_) => {
            let mut cursor = start;
            while let Some(byte) = input.get(cursor) {
                if matches!(byte, b',' | b']' | b'}' | b' ' | b'\n' | b'\r' | b'\t') {
                    break;
                }
                cursor += 1;
            }
            if cursor == start {
                Err("empty JSON scalar".into())
            } else {
                Ok(cursor)
            }
        }
        None => Err("expected JSON value, found end of input".into()),
    }
}

fn compound_value_end(input: &[u8], start: usize) -> Result<usize, String> {
    let mut stack = Vec::new();
    let mut cursor = start;
    let mut in_string = false;
    let mut escaped = false;

    while let Some(byte) = input.get(cursor).copied() {
        if in_string {
            if escaped {
                escaped = false;
            } else {
                match byte {
                    b'\\' => escaped = true,
                    b'"' => in_string = false,
                    _ => {}
                }
            }
            cursor += 1;
            continue;
        }

        match byte {
            b'"' => in_string = true,
            b'{' => stack.push(b'}'),
            b'[' => stack.push(b']'),
            b'}' | b']' => {
                let expected = stack
                    .pop()
                    .ok_or_else(|| "unexpected closing delimiter".to_owned())?;
                if byte != expected {
                    return Err("mismatched JSON delimiters".into());
                }
                if stack.is_empty() {
                    return Ok(cursor + 1);
                }
            }
            _ => {}
        }
        cursor += 1;
    }

    Err("unterminated compound JSON value".into())
}

fn string_end(input: &[u8], start: usize) -> Result<usize, String> {
    let mut cursor = start + 1;
    let mut escaped = false;

    while let Some(byte) = input.get(cursor).copied() {
        if escaped {
            escaped = false;
        } else {
            match byte {
                b'\\' => escaped = true,
                b'"' => return Ok(cursor + 1),
                _ => {}
            }
        }
        cursor += 1;
    }

    Err("unterminated JSON string".into())
}

fn skip_whitespace(input: &[u8], mut cursor: usize) -> usize {
    while input
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
    {
        cursor += 1;
    }
    cursor
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|candidate| candidate == needle)
}

#[cfg(test)]
mod tests {
    use super::{
        contains_bytes, item_ranges, STOCK_REALTIME_FORMAT_FIELD, STOCK_SNAPSHOT_FORMAT_FIELD,
    };

    #[test]
    fn preserves_selected_value_lexemes_exactly() {
        let page = br#"{"next_cursor":"secret","items":[{"price":2320.0},{"text":"},[]"}]}"#;
        let ranges = item_ranges(page).unwrap();

        assert_eq!(&page[ranges[0].clone()], br#"{"price":2320.0}"#);
        assert_eq!(&page[ranges[1].clone()], br#"{"text":"},[]"}"#);
    }

    #[test]
    fn selects_both_exact_regular_stock_wire_formats() {
        let snapshot = br#"{"format":"STOCK_SNAPSHOT","price":2320.0}"#;
        let realtime = br#"{"format":"STOCK_REALTIME","price":2320.0}"#;
        let odd_lot = br#"{"format":"INTRADAY_ODDLOT_REALTIME","price":2320.0}"#;

        assert!(contains_bytes(snapshot, STOCK_SNAPSHOT_FORMAT_FIELD));
        assert!(contains_bytes(realtime, STOCK_REALTIME_FORMAT_FIELD));
        assert!(!contains_bytes(odd_lot, STOCK_SNAPSHOT_FORMAT_FIELD));
        assert!(!contains_bytes(odd_lot, STOCK_REALTIME_FORMAT_FIELD));
    }
}
