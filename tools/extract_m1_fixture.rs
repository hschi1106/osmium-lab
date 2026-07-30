use std::{
    env,
    error::Error,
    fs::{self, File},
    io::{self, Write},
    ops::Range,
    path::PathBuf,
};

const EXTRACTOR_VERSION: u16 = 1;
const SELECTIONS: [(&str, &[usize]); 2] = [
    ("0001.json", &[179, 180, 183, 329, 416]),
    ("0016.json", &[2201, 2202, 2204]),
];

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os().skip(1);
    let source_root = PathBuf::from(
        arguments
            .next()
            .ok_or("usage: extract_m1_fixture <source-complete-directory> <output-jsonl>")?,
    );
    let output_path = PathBuf::from(
        arguments
            .next()
            .ok_or("usage: extract_m1_fixture <source-complete-directory> <output-jsonl>")?,
    );
    if arguments.next().is_some() {
        return Err("extract_m1_fixture accepts exactly two arguments".into());
    }

    let mut output = File::create(&output_path)?;
    let mut record_count = 0_usize;

    for (page_name, selected_indices) in SELECTIONS {
        let page_path = source_root.join("pages").join(page_name);
        let page = fs::read(&page_path)?;
        let item_ranges = item_ranges(&page).map_err(|message| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {message}", page_path.display()),
            )
        })?;

        for selected_index in selected_indices {
            let range = item_ranges.get(*selected_index).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{} does not contain items[{selected_index}]",
                        page_path.display()
                    ),
                )
            })?;
            let record = &page[range.clone()];
            if record.first() != Some(&b'{') || record.last() != Some(&b'}') {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{} items[{selected_index}] is not a JSON object",
                        page_path.display()
                    ),
                )
                .into());
            }

            output.write_all(record)?;
            output.write_all(b"\n")?;
            record_count += 1;
        }
    }

    output.flush()?;
    println!("extractor_version={EXTRACTOR_VERSION} records={record_count}");
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

#[cfg(test)]
mod tests {
    use super::item_ranges;

    #[test]
    fn preserves_selected_value_lexemes_exactly() {
        let page = br#"{"next_cursor":"secret","items":[{"price":2320.0},{"text":"},[]"}]}"#;
        let ranges = item_ranges(page).unwrap();

        assert_eq!(&page[ranges[0].clone()], br#"{"price":2320.0}"#);
        assert_eq!(&page[ranges[1].clone()], br#"{"text":"},[]"}"#);
    }
}
