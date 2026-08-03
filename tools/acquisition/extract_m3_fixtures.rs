use std::{env, error::Error, fs, io, ops::Range, path::Path};

const EXTRACTOR_VERSION: u16 = 1;
const TRADING_DATE: &str = "2026-07-20";
const MARKET_FIELD: &[u8] = br#""market":"taifex_fut""#;

struct Selection {
    symbol: &'static str,
    source_segment: &'static str,
    fixture_segment: &'static str,
    pages: &'static [&'static str],
    expected_records: usize,
}

const SELECTIONS: &[Selection] = &[
    Selection {
        symbol: "TXFH6",
        source_segment: "after_hours",
        fixture_segment: "after-hours",
        pages: &["0001", "0052", "0053", "0079"],
        expected_records: 16_872,
    },
    Selection {
        symbol: "TXFH6",
        source_segment: "regular",
        fixture_segment: "regular",
        pages: &["0001", "0020", "0039"],
        expected_records: 12_667,
    },
    Selection {
        symbol: "CDFH6",
        source_segment: "after_hours",
        fixture_segment: "after-hours",
        pages: &["0001", "0009", "0010", "0014"],
        expected_records: 17_866,
    },
    Selection {
        symbol: "CDFH6",
        source_segment: "regular",
        fixture_segment: "regular",
        pages: &["0001", "0008", "0016"],
        expected_records: 12_510,
    },
    Selection {
        symbol: "CAFH6",
        source_segment: "regular",
        fixture_segment: "regular",
        pages: &["0001", "0004", "0008"],
        expected_records: 14_299,
    },
];

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os().skip(1);
    let source_root = arguments.next().ok_or(
        "usage: extract_m3_fixtures <source-evidence-directory> <fixture-output-directory>",
    )?;
    let output_root = arguments.next().ok_or(
        "usage: extract_m3_fixtures <source-evidence-directory> <fixture-output-directory>",
    )?;
    if arguments.next().is_some() {
        return Err("extract_m3_fixtures accepts exactly two arguments".into());
    }
    let source_root = Path::new(&source_root);
    let output_root = Path::new(&output_root);
    if output_root.exists() {
        return Err(format!(
            "refusing to overwrite fixture output: {}",
            output_root.display()
        )
        .into());
    }

    let mut total_records = 0;
    for selection in SELECTIONS {
        let source_pages = source_root
            .join("partitions")
            .join(selection.symbol)
            .join(selection.source_segment)
            .join("pages");
        let output = output_root
            .join(selection.symbol)
            .join(TRADING_DATE)
            .join(selection.fixture_segment);
        fs::create_dir_all(&output)?;

        let symbol_field = format!(r#""symbol":"{}""#, selection.symbol);
        let mut partition_records = 0;
        for page_number in selection.pages {
            let page_name = format!("{page_number}.json");
            let page_path = source_pages.join(&page_name);
            let page = fs::read(&page_path)?;
            let mut shard = Vec::new();
            let ranges = item_ranges(&page).map_err(|message| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: {message}", page_path.display()),
                )
            })?;

            for (item_index, range) in ranges.into_iter().enumerate() {
                let record = &page[range];
                if record.first() != Some(&b'{') || record.last() != Some(&b'}') {
                    return Err(invalid_record(&page_path, item_index, "not a JSON object").into());
                }
                if !contains_bytes(record, MARKET_FIELD) {
                    return Err(
                        invalid_record(&page_path, item_index, "market is not taifex_fut").into(),
                    );
                }
                if !contains_bytes(record, symbol_field.as_bytes()) {
                    return Err(
                        invalid_record(&page_path, item_index, "symbol does not match").into(),
                    );
                }

                shard.extend_from_slice(record);
                shard.push(b'\n');
                partition_records += 1;
            }

            fs::write(output.join(format!("{page_number}.jsonl")), shard)?;
        }

        if partition_records != selection.expected_records {
            return Err(format!(
                "{} {} expected {} records, found {partition_records}",
                selection.symbol, selection.source_segment, selection.expected_records
            )
            .into());
        }
        total_records += partition_records;
        println!(
            "{} {}: files={} records={partition_records}",
            selection.symbol,
            selection.source_segment,
            selection.pages.len()
        );
    }

    println!("extractor_version={EXTRACTOR_VERSION} total_records={total_records}");
    Ok(())
}

fn invalid_record(path: &Path, item_index: usize, message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{} items[{item_index}] {message}", path.display()),
    )
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
    use super::{contains_bytes, item_ranges, MARKET_FIELD};

    #[test]
    fn preserves_item_bytes_and_numeric_lexemes() {
        let page = br#"{"next_cursor":"secret","items":[{"price":2320.0},{"text":"},[]"}]}"#;
        let ranges = item_ranges(page).unwrap();

        assert_eq!(&page[ranges[0].clone()], br#"{"price":2320.0}"#);
        assert_eq!(&page[ranges[1].clone()], br#"{"text":"},[]"}"#);
    }

    #[test]
    fn recognizes_only_the_exact_market_field() {
        assert!(contains_bytes(br#"{"market":"taifex_fut"}"#, MARKET_FIELD));
        assert!(!contains_bytes(br#"{"market":"taifex_opt"}"#, MARKET_FIELD));
    }
}
