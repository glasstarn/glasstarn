//! Reading Apache Parquet files.
//!
//! # Why Parquet matters here
//!
//! Parquet stores **types**, not text. A timestamp column is already a
//! timestamp, so the hardest ingest problem in this product simply does not
//! arise: there is no `05/01/2026` to interpret, no mixed formats, no day
//! versus month question. Everything the timestamp machinery exists to handle
//! is answered by the file itself.
//!
//! It is also far smaller and faster to read than the equivalent CSV, which is
//! what makes larger logs practical.
//!
//! # How values are handed on
//!
//! Values are rendered into the same [`Row`] shape CSV produces, so every
//! downstream stage is unchanged. Timestamps are rendered in unambiguous ISO
//! form, which detection then resolves immediately and without a question.
//! Rendering rather than passing typed values through is a deliberate
//! simplification: one representation downstream, and the type information is
//! preserved in practice because the rendered form is unambiguous.

use crate::read::{ReadError, Row, Schema, Table};
use arrow_array::{Array, RecordBatch};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use std::path::Path;
use std::rc::Rc;

/// Why a Parquet file could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ParquetError {
    #[error("could not open {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not readable as Parquet: {source}")]
    Format {
        path: String,
        #[source]
        source: parquet::errors::ParquetError,
    },
}

impl From<ParquetError> for ReadError {
    fn from(error: ParquetError) -> Self {
        ReadError::NoHeader {
            path: error.to_string(),
        }
    }
}

/// Whether a path looks like a Parquet file.
pub fn is_parquet(path: impl AsRef<Path>) -> bool {
    path.as_ref()
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("parquet") || e.eq_ignore_ascii_case("pq"))
}

/// Opens a Parquet file and returns its schema plus a lazy row iterator.
///
/// Rows are produced batch by batch, so the whole file is never held.
pub fn stream_parquet(path: impl AsRef<Path>) -> Result<crate::read::RowStream, ParquetError> {
    let path = path.as_ref();
    let display = path.display().to_string();

    let file = std::fs::File::open(path).map_err(|source| ParquetError::Io {
        path: display.clone(),
        source,
    })?;
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(file).map_err(|source| ParquetError::Format {
            path: display.clone(),
            source,
        })?;

    let headers: Vec<String> = builder
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    let width = headers.len();
    let schema = Rc::new(Schema::new(headers));
    let row_schema = Rc::clone(&schema);

    let reader = builder.build().map_err(|source| ParquetError::Format {
        path: display.clone(),
        source,
    })?;

    // Each batch is expanded to rows as it arrives, then dropped.
    let mut emitted = 0usize;
    let rows = reader.flat_map(move |batch| match batch {
        Ok(batch) => {
            let mut out = Vec::with_capacity(batch.num_rows());
            for r in 0..batch.num_rows() {
                let mut values = Vec::with_capacity(width);
                for c in 0..width.min(batch.num_columns()) {
                    values.push(render(batch.column(c), r));
                }
                emitted += 1;
                out.push(Ok(Row::new(Rc::clone(&row_schema), values, emitted + 1)));
            }
            out
        }
        Err(source) => vec![Err(ReadError::Csv {
            path: display.clone(),
            source: csv::Error::from(std::io::Error::other(source.to_string())),
        })],
    });

    Ok((schema, Box::new(rows)))
}

/// Reads a Parquet file into the same table shape CSV produces.
pub fn read_parquet(path: impl AsRef<Path>) -> Result<Table, ParquetError> {
    let path = path.as_ref();
    let display = path.display().to_string();

    let file = std::fs::File::open(path).map_err(|source| ParquetError::Io {
        path: display.clone(),
        source,
    })?;
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(file).map_err(|source| ParquetError::Format {
            path: display.clone(),
            source,
        })?;

    let headers: Vec<String> = builder
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    let schema = Rc::new(Schema::new(headers.clone()));

    let reader = builder.build().map_err(|source| ParquetError::Format {
        path: display.clone(),
        source,
    })?;

    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch.map_err(|source| ParquetError::Format {
            path: display.clone(),
            source: source.into(),
        })?;
        append_batch(&batch, &schema, headers.len(), &mut rows);
    }

    Ok(Table { schema, rows })
}

fn append_batch(batch: &RecordBatch, schema: &Rc<Schema>, width: usize, rows: &mut Vec<Row>) {
    let start = rows.len();
    for r in 0..batch.num_rows() {
        let mut values = Vec::with_capacity(width);
        for c in 0..width.min(batch.num_columns()) {
            values.push(render(batch.column(c), r));
        }
        // 1-based, counting a header row, so evidence numbering matches CSV.
        rows.push(Row::new(Rc::clone(schema), values, start + r + 2));
    }
}

/// Renders one cell as text.
///
/// Null becomes an empty string, matching how a blank CSV cell arrives, so the
/// same "did this happen or is it missing?" decision applies either way.
fn render(column: &dyn Array, row: usize) -> String {
    use arrow_array::cast::AsArray;
    use arrow_array::types::*;

    if column.is_null(row) {
        return String::new();
    }

    let dt = column.data_type();
    use arrow_schema::{DataType, TimeUnit};

    match dt {
        DataType::Utf8 => column.as_string::<i32>().value(row).to_owned(),
        DataType::LargeUtf8 => column.as_string::<i64>().value(row).to_owned(),
        DataType::Boolean => column.as_boolean().value(row).to_string(),
        DataType::Int8 => column.as_primitive::<Int8Type>().value(row).to_string(),
        DataType::Int16 => column.as_primitive::<Int16Type>().value(row).to_string(),
        DataType::Int32 => column.as_primitive::<Int32Type>().value(row).to_string(),
        DataType::Int64 => column.as_primitive::<Int64Type>().value(row).to_string(),
        DataType::UInt8 => column.as_primitive::<UInt8Type>().value(row).to_string(),
        DataType::UInt16 => column.as_primitive::<UInt16Type>().value(row).to_string(),
        DataType::UInt32 => column.as_primitive::<UInt32Type>().value(row).to_string(),
        DataType::UInt64 => column.as_primitive::<UInt64Type>().value(row).to_string(),
        DataType::Float32 => column.as_primitive::<Float32Type>().value(row).to_string(),
        DataType::Float64 => column.as_primitive::<Float64Type>().value(row).to_string(),
        DataType::Date32 => render_date(
            column.as_primitive::<Date32Type>().value(row) as i64,
            86_400_000,
        ),
        DataType::Date64 => render_date(column.as_primitive::<Date64Type>().value(row), 1),
        DataType::Timestamp(unit, _) => {
            let raw = match unit {
                TimeUnit::Second => column.as_primitive::<TimestampSecondType>().value(row),
                TimeUnit::Millisecond => {
                    column.as_primitive::<TimestampMillisecondType>().value(row)
                }
                TimeUnit::Microsecond => {
                    column.as_primitive::<TimestampMicrosecondType>().value(row)
                }
                TimeUnit::Nanosecond => column.as_primitive::<TimestampNanosecondType>().value(row),
            };
            let millis = match unit {
                TimeUnit::Second => raw.saturating_mul(1_000),
                TimeUnit::Millisecond => raw,
                TimeUnit::Microsecond => raw / 1_000,
                TimeUnit::Nanosecond => raw / 1_000_000,
            };
            render_date(millis, 1)
        }
        // Anything else is handed on as its debug form rather than dropped;
        // an unexpected type should be visible, not silently blank.
        _ => format!("{:?}", column.slice(row, 1)),
    }
}

/// Renders an epoch value as an unambiguous ISO timestamp.
fn render_date(value: i64, scale: i64) -> String {
    let millis = value.saturating_mul(scale);
    match chrono::DateTime::from_timestamp_millis(millis) {
        Some(dt) => dt.naive_utc().format("%Y-%m-%d %H:%M:%S").to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{Int64Array, RecordBatch, StringArray, TimestampMillisecondArray};
    use arrow_schema::{DataType, Field, TimeUnit};
    use parquet::arrow::ArrowWriter;
    use std::sync::Arc;

    fn write_sample(name: &str) -> std::path::PathBuf {
        let schema = Arc::new(arrow_schema::Schema::new(vec![
            Field::new("case_id", DataType::Utf8, false),
            Field::new("activity", DataType::Utf8, false),
            Field::new(
                "timestamp",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                false,
            ),
            Field::new("amount", DataType::Int64, true),
        ]));

        // 2026-03-02 09:00:00 UTC and two hours later.
        let base = 1_772_442_000_000i64;
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(StringArray::from(vec!["C1", "C1", "C2"])),
                Arc::new(StringArray::from(vec!["Receive", "Ship", "Receive"])),
                Arc::new(TimestampMillisecondArray::from(vec![
                    base,
                    base + 7_200_000,
                    base + 3_600_000,
                ])),
                Arc::new(Int64Array::from(vec![Some(100), None, Some(250)])),
            ],
        )
        .unwrap();

        let path = std::env::temp_dir().join(format!("pm_parquet_{name}.parquet"));
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        path
    }

    #[test]
    fn recognises_parquet_paths() {
        assert!(is_parquet("log.parquet"));
        assert!(is_parquet("LOG.PARQUET"));
        assert!(!is_parquet("log.csv"));
        assert!(!is_parquet("log"));
    }

    #[test]
    fn reads_columns_and_rows() {
        let p = write_sample("basic");
        let t = read_parquet(&p).unwrap();
        assert_eq!(t.headers(), ["case_id", "activity", "timestamp", "amount"]);
        assert_eq!(t.rows.len(), 3);
        assert_eq!(t.rows[0].get("case_id"), "C1");
        assert_eq!(t.rows[0].get("activity"), "Receive");
    }

    /// The point of Parquet: a stored timestamp needs no interpretation.
    #[test]
    fn renders_timestamps_unambiguously() {
        let p = write_sample("timestamps");
        let t = read_parquet(&p).unwrap();
        let stamps: Vec<&str> = t.rows.iter().map(|r| r.get("timestamp")).collect();
        assert_eq!(stamps[0], "2026-03-02 09:00:00");
        assert_eq!(stamps[1], "2026-03-02 11:00:00");

        let detection =
            crate::timestamp::detect(&stamps.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        assert!(
            detection.ambiguity.is_none(),
            "a typed timestamp must never be ambiguous"
        );
        assert!(detection.is_conclusive());
    }

    /// A null must arrive the same way a blank CSV cell does, so the same
    /// decision applies either way.
    #[test]
    fn renders_nulls_as_blank() {
        let p = write_sample("nulls");
        let t = read_parquet(&p).unwrap();
        assert_eq!(t.rows[1].get("amount"), "");
        assert_eq!(t.rows[0].get("amount"), "100");
    }

    #[test]
    fn numbers_rows_as_csv_does() {
        let p = write_sample("rownum");
        let t = read_parquet(&p).unwrap();
        assert_eq!(t.rows[0].source_row, 2);
        assert_eq!(t.rows[2].source_row, 4);
    }

    /// The whole pipeline must accept a Parquet table unchanged.
    #[test]
    fn flows_through_validation_and_normalization() {
        use crate::finding::Policy;
        let p = write_sample("pipeline");
        let t = read_parquet(&p).unwrap();
        let mapping = crate::validate::Mapping::new("case_id", "activity", "timestamp");
        let report = crate::validate::validate(&t.rows, &mapping);
        let n = crate::normalize::normalize(&t.rows, &mapping, &report, &Policy::new()).unwrap();
        assert_eq!(n.log.event_count(), 3);
        assert_eq!(n.log.case_count(), 2);
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        assert!(read_parquet("/nonexistent/log.parquet").is_err());
    }

    #[test]
    fn a_non_parquet_file_is_rejected_clearly() {
        let path = std::env::temp_dir().join("pm_parquet_not_parquet.parquet");
        std::fs::write(&path, b"case_id,activity\nC1,A\n").unwrap();
        assert!(matches!(
            read_parquet(&path),
            Err(ParquetError::Format { .. })
        ));
    }

    /// Streaming and eager reading must agree exactly.
    #[test]
    fn streaming_matches_eager_reading() {
        let p = write_sample("stream");
        let eager = read_parquet(&p).unwrap();
        let (_, rows) = stream_parquet(&p).unwrap();
        let streamed: Vec<Row> = rows.map(Result::unwrap).collect();
        assert_eq!(streamed, eager.rows);
    }

    #[test]
    fn is_deterministic() {
        let p = write_sample("determinism");
        assert_eq!(read_parquet(&p).unwrap(), read_parquet(&p).unwrap());
    }
}
