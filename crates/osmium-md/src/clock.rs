use arrow_array::types::{Int64Type, TimestampMicrosecondType};

use crate::{
    MdError,
    parquet_io::{ParquetFile, optional, primitive},
};

/// Observed time of each `selected` seq over the whole feed.
///
/// The observed time of message `s` is the largest `exch_ts` among every message the
/// feed disseminated with `seq <= s`, including symbols outside the selection, so it
/// does not depend on which symbols a run replays. `selected` must be sorted and
/// unique; the result is aligned with it.
pub(crate) fn observed_times(
    files: impl IntoIterator<Item = ParquetFile>,
    selected: &[u64],
) -> Result<Vec<i64>, MdError> {
    let mut bucket_max = vec![i64::MIN; selected.len()];
    let Some(&last) = selected.last() else {
        return Ok(bucket_max);
    };
    for file in files {
        file.read(&["seq", "exch_ts"], None, |path, batch| {
            let seq = primitive::<Int64Type>(path, batch, "seq")?;
            let exch_ts = primitive::<TimestampMicrosecondType>(path, batch, "exch_ts")?;
            for row in 0..batch.num_rows() {
                let (Some(seq), Some(time)) = (optional(seq, row), optional(exch_ts, row)) else {
                    return Err(MdError::inconsistent(path, "seq or exch_ts is null"));
                };
                let Ok(seq) = u64::try_from(seq) else {
                    return Err(MdError::inconsistent(path, "seq is negative"));
                };
                if seq > last {
                    continue;
                }
                // Message `seq` is visible to every selected seq at or after it.
                let bucket = selected.partition_point(|&selected| selected < seq);
                let slot = &mut bucket_max[bucket];
                *slot = (*slot).max(time);
            }
            Ok(())
        })?;
    }
    let mut running = i64::MIN;
    for slot in &mut bucket_max {
        running = running.max(*slot);
        *slot = running;
    }
    Ok(bucket_max)
}
