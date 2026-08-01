use std::{
    collections::BTreeMap,
    env, error::Error, fs,
    io::{self, Write},
    ops::Range,
    path::{Path, PathBuf},
};

const EXTRACTOR_VERSION: u16 = 1;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let raw_root = PathBuf::from(args.next().ok_or(
        "usage: extract_m5_fixtures <raw-root> <fixture-root>",
    )?);
    let fixture_root = PathBuf::from(args.next().ok_or(
        "usage: extract_m5_fixtures <raw-root> <fixture-root>",
    )?);
    if args.next().is_some() {
        return Err("extract_m5_fixtures accepts exactly two arguments".into());
    }

    extract_warrant(&raw_root, &fixture_root)?;
    extract_future(&raw_root, &fixture_root)?;
    extract_option(&raw_root, &fixture_root)?;
    println!("extractor_version={EXTRACTOR_VERSION}");
    Ok(())
}

fn extract_warrant(raw_root: &Path, fixture_root: &Path) -> Result<(), Box<dyn Error>> {
    let raw = raw_root.join("twse/2026-07-20/03003T/complete");
    let output = fixture_root.join("twse/03003T/2026-07-20");
    ensure_new(&output)?;
    let counts = copy_pages(
        &raw.join("partitions/regular/pages"),
        &output.join("regular-quotes"),
        "twse",
        "03003T",
        None,
    )?;
    copy_daily(&raw.join("discovery/instrument.json"), &output.join("daily.json"))?;
    write_metadata(
        &output,
        &[
            ("source_market", "twse"),
            ("market", "twse"),
            ("instrument_kind", "warrant"),
            ("symbol", "03003T"),
            ("trading_date", "2026-07-20"),
            ("sessions", "[regular]"),
            ("source_formats", &format_counts(&counts)),
            ("raw_acquisition", "raw/teralion/twse/2026-07-20/03003T/complete"),
            ("official_protocol", "https://www.twse.com.tw/zh/products/securities/warrant/mops.html"),
            ("reference_underlying", "2454"),
            ("reference_underlying_name", "聯發科"),
            ("reference_option_side", "put"),
            ("reference_strike", "1630.26"),
            ("reference_expiry", "2026-10-16"),
            ("reference_currency", "TWD"),
            ("reference_multiplier", "1"),
            ("reference_quantity_unit", "trading_unit"),
            ("reference_units_per_trading_unit", "1000"),
            ("reference_provenance", "TWSE warrant OpenAPI t187ap37_L observed 2026-07-31; Teralion daily 2026-07-20"),
        ],
    )?;
    println!("warrant_records={}", counts.values().sum::<usize>());
    Ok(())
}

fn extract_future(raw_root: &Path, fixture_root: &Path) -> Result<(), Box<dyn Error>> {
    let raw = raw_root.join("taifex/2026-07-28/evidence");
    let output = fixture_root.join("taifex/TXFH6/2026-07-28");
    ensure_new(&output)?;
    let mut counts = BTreeMap::new();
    for (source_segment, fixture_segment) in
        [("after_hours", "after-hours"), ("regular", "regular")]
    {
        merge_counts(
            &mut counts,
            copy_pages(
                &raw.join(format!("partitions/TXFH6/{source_segment}/pages")),
                &output.join(fixture_segment),
                "taifex_fut",
                "TXFH6",
                None,
            )?,
        );
    }
    copy_daily(
        &raw.join("discovery/instrument-TXFH6.json"),
        &output.join("daily.json"),
    )?;
    write_metadata(
        &output,
        &[
            ("source_market", "taifex_fut"),
            ("market", "taifex"),
            ("instrument_kind", "future"),
            ("symbol", "TXFH6"),
            ("trading_date", "2026-07-28"),
            ("sessions", "[after_hours, regular]"),
            ("source_formats", &format_counts(&counts)),
            ("raw_acquisition", "raw/teralion/taifex/2026-07-28/evidence"),
            ("official_protocol", "https://www.taifex.com.tw/file/taifex/eng/eng11/TechDocs/19/Market_Data_Transmission_Manual_v2.31.0S.pdf"),
            ("reference_provenance", "existing M3 TAIFEX complete source evidence"),
        ],
    )?;
    println!("future_records={}", counts.values().sum::<usize>());
    Ok(())
}

fn extract_option(raw_root: &Path, fixture_root: &Path) -> Result<(), Box<dyn Error>> {
    let raw = raw_root.join("taifex_opt/2026-07-28/TXO24000U6/complete");
    let output = fixture_root.join("taifex/TXO24000U6/2026-07-28");
    ensure_new(&output)?;
    let counts = copy_pages(
        &raw.join("partitions/combined/pages"),
        &output,
        "taifex_opt",
        "TXO24000U6",
        Some(("2026-07-28T05:05:00+08:00", "after-hours", "regular")),
    )?;
    copy_daily(
        &raw.join("discovery/instrument.json"),
        &output.join("daily.json"),
    )?;
    write_metadata(
        &output,
        &[
            ("source_market", "taifex_opt"),
            ("market", "taifex"),
            ("instrument_kind", "option"),
            ("symbol", "TXO24000U6"),
            ("trading_date", "2026-07-28"),
            ("sessions", "[after_hours, regular]"),
            ("source_formats", &format_counts(&counts)),
            ("known_skipped_formats", "[I021, I023, I030, I070, I072]"),
            ("raw_acquisition", "raw/teralion/taifex_opt/2026-07-28/TXO24000U6/complete"),
            ("official_protocol", "https://www.taifex.com.tw/file/taifex/eng/eng11/TechDocs/19/Market_Data_Transmission_Manual_v2.31.0S.pdf"),
            ("official_product", "https://www.taifex.com.tw/enl/eng2/tXO"),
            ("reference_underlying", "TAIEX"),
            ("reference_option_side", "put"),
            ("reference_strike", "24000"),
            ("reference_expiry", "2026-09-16"),
            ("reference_currency", "TWD"),
            ("reference_multiplier", "50"),
            ("reference_quantity_unit", "contract"),
            ("reference_units_per_trading_unit", "1"),
            ("reference_provenance", "TAIFEX TXO contract specification plus symbol month/put code and Teralion daily expiry month"),
        ],
    )?;
    println!("option_records={}", counts.values().sum::<usize>());
    Ok(())
}

fn ensure_new(path: &Path) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        return Err(format!("refusing to overwrite fixture partition: {}", path.display()).into());
    }
    fs::create_dir_all(path)?;
    Ok(())
}

fn copy_daily(raw: &Path, output: &Path) -> Result<(), Box<dyn Error>> {
    fs::write(output, fs::read(raw)?)?;
    Ok(())
}

fn copy_pages(
    source: &Path,
    default_output: &Path,
    expected_market: &str,
    expected_symbol: &str,
    split: Option<(&str, &str, &str)>,
) -> Result<BTreeMap<String, usize>, Box<dyn Error>> {
    let mut pages = fs::read_dir(source)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|value| value == "json"))
        .collect::<Vec<_>>();
    pages.sort();
    if pages.is_empty() {
        return Err(format!("no raw pages in {}", source.display()).into());
    }
    let mut counts = BTreeMap::new();
    for page in pages {
        let page_number = page
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or("raw page has no numeric stem")?;
        let bytes = fs::read(&page)?;
        let ranges = item_ranges(&bytes).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {error}", page.display()),
            )
        })?;
        let mut output = BTreeMap::<String, Vec<u8>>::new();
        for range in ranges {
            let record = &bytes[range];
            let market_field = format!(r#""market":"{expected_market}""#);
            let symbol_field = format!(r#""symbol":"{expected_symbol}""#);
            if !contains_bytes(record, market_field.as_bytes())
                || !contains_bytes(record, symbol_field.as_bytes())
            {
                return Err(format!("{} contains an identity mismatch", page.display()).into());
            }
            let format = json_string_field(record, "format")?;
            let segment = match split {
                Some((after_end, after, regular)) => {
                    let match_time = json_string_field(record, "match_time")?;
                    if match_time.as_str() < after_end {
                        after
                    } else {
                        regular
                    }
                }
                None => default_output
                    .file_name()
                    .and_then(|value| value.to_str())
                    .ok_or("fixture output has no directory name")?,
            };
            let entry = output.entry(segment.to_owned()).or_default();
            entry.extend_from_slice(record);
            entry.push(b'\n');
            *counts.entry(format).or_insert(0) += 1;
        }
        for (segment, contents) in output {
            let directory = if split.is_some() {
                default_output.join(segment)
            } else {
                default_output.to_path_buf()
            };
            fs::create_dir_all(&directory)?;
            let mut file = fs::File::create(directory.join(format!("{page_number}.jsonl")))?;
            file.write_all(&contents)?;
        }
    }
    Ok(counts)
}

fn merge_counts(target: &mut BTreeMap<String, usize>, source: BTreeMap<String, usize>) {
    for (format, count) in source {
        *target.entry(format).or_insert(0) += count;
    }
}

fn format_counts(counts: &BTreeMap<String, usize>) -> String {
    let fields = counts
        .iter()
        .map(|(format, count)| format!("{format}: {count}"))
        .collect::<Vec<_>>();
    format!("[{}]", fields.join(", "))
}

fn write_metadata(path: &Path, fields: &[(&str, &str)]) -> Result<(), Box<dyn Error>> {
    let mut output =
        String::from("schema_version: 1\nredistribution: private-internal-review-only\n");
    for (key, value) in fields {
        output.push_str(key);
        output.push_str(": ");
        output.push_str(value);
        output.push('\n');
    }
    fs::write(path.join("metadata.yaml"), output)?;
    Ok(())
}

fn json_string_field(record: &[u8], field: &str) -> Result<String, Box<dyn Error>> {
    let needle = format!("\"{}\":\"", field);
    let start = record
        .windows(needle.len())
        .position(|window| window == needle.as_bytes())
        .ok_or_else(|| format!("missing {field}"))?
        + needle.len();
    let end = record[start..]
        .iter()
        .position(|byte| *byte == b'"')
        .ok_or_else(|| format!("unterminated {field}"))?
        + start;
    Ok(String::from_utf8(record[start..end].to_vec())?)
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
            Some(byte) => return Err(format!("expected ',' or ']', found 0x{byte:02x}")),
            None => return Err("unterminated items array".into()),
        }
    }
}

fn find_items_array(page: &[u8]) -> Result<usize, String> {
    let key = b"\"items\"";
    for start in 0..page.len().saturating_sub(key.len() - 1) {
        if page.get(start..start + key.len()) != Some(key) {
            continue;
        }
        let colon = skip_whitespace(page, start + key.len());
        if page.get(colon) != Some(&b':') {
            continue;
        }
        let array = skip_whitespace(page, colon + 1);
        if page.get(array) == Some(&b'[') {
            return Ok(array);
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
        None => Err("expected JSON value at end of input".into()),
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
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            cursor += 1;
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => stack.push(b'}'),
            b'[' => stack.push(b']'),
            b'}' | b']' => {
                let expected = stack.pop().ok_or("unexpected closing delimiter")?;
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
    Err("unterminated JSON compound value".into())
}

fn string_end(input: &[u8], start: usize) -> Result<usize, String> {
    let mut cursor = start + 1;
    let mut escaped = false;
    while let Some(byte) = input.get(cursor).copied() {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            return Ok(cursor + 1);
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
