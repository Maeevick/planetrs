//! Tables of plain values in and out of Parquet, the one place the `parquet` and `arrow` crates are
//! named, so that swapping them touches nothing else.
//!
//! A table is a list of row groups, each a list of named columns of equal length. The caller cuts
//! the row groups, because only the caller knows where a commune or a multipolygon ends, and every
//! setting that shapes the bytes is pinned here, so that the same table always writes the same file.

use std::sync::Arc;

use arrow_array::builder::{ListBuilder, StringBuilder};
use arrow_array::cast::AsArray;
use arrow_array::types::{Float64Type, Int32Type, Int64Type};
use arrow_array::{
    Array, ArrayRef, BinaryArray, Float64Array, Int32Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use bytes::Bytes;
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::arrow_writer::ArrowWriterOptions;
use parquet::basic::{Compression, GzipLevel};
use parquet::file::metadata::KeyValue;
use parquet::file::properties::{EnabledStatistics, WriterProperties};

/// The writer every file of a dataset names, rather than the library that happens to encode it.
const WRITER: &str = concat!("planetrs-dataset version ", env!("CARGO_PKG_VERSION"));

/// GZIP at its usual level: no C library, and readable by every Parquet reader there is.
const GZIP_LEVEL: u32 = 6;

/// The values of one column, each kind as Parquet stores it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Values {
    /// 64-bit integers, none missing.
    Integers(Vec<i64>),
    /// 32-bit integers, none missing.
    SmallIntegers(Vec<i32>),
    /// 64-bit integers, some missing.
    OptionalIntegers(Vec<Option<i64>>),
    /// Doubles, some missing.
    OptionalReals(Vec<Option<f64>>),
    /// UTF-8 texts, some missing.
    OptionalTexts(Vec<Option<String>>),
    /// Lists of UTF-8 texts, each list present and possibly empty.
    TextLists(Vec<Vec<String>>),
    /// Byte strings, some missing.
    OptionalBytes(Vec<Option<Vec<u8>>>),
}

/// One named column.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Column {
    pub(crate) name: String,
    pub(crate) values: Values,
}

impl Column {
    pub(crate) fn of(name: &str, values: Values) -> Self {
        Self {
            name: name.to_string(),
            values,
        }
    }
}

/// Why a table could not be written or read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TableError {
    /// The table could not be written as Parquet.
    Unwritable {
        /// What the Parquet writer said.
        detail: String,
    },
    /// The bytes could not be read as the table expected.
    Unreadable {
        /// What went wrong.
        detail: String,
    },
}

impl std::fmt::Display for TableError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unwritable { detail } => write!(formatter, "the table could not be written: {detail}"),
            Self::Unreadable { detail } => write!(formatter, "the table could not be read: {detail}"),
        }
    }
}

impl std::error::Error for TableError {}

fn unwritable(error: impl std::fmt::Display) -> TableError {
    TableError::Unwritable {
        detail: error.to_string(),
    }
}

fn unreadable(error: impl std::fmt::Display) -> TableError {
    TableError::Unreadable {
        detail: error.to_string(),
    }
}

/// The row groups as one Parquet file, with `metadata` among its key-value pairs.
pub(crate) fn write(row_groups: &[Vec<Column>], metadata: &[(&str, String)]) -> Result<Vec<u8>, TableError> {
    let first = row_groups.first().map(Vec::as_slice).unwrap_or_default();
    let schema = Arc::new(Schema::new(first.iter().map(field_of).collect::<Vec<_>>()));
    let properties = WriterProperties::builder()
        .set_created_by(WRITER.to_string())
        .set_compression(Compression::GZIP(
            GzipLevel::try_new(GZIP_LEVEL).map_err(unwritable)?,
        ))
        .set_statistics_enabled(EnabledStatistics::Chunk)
        .set_max_row_group_row_count(None)
        .set_max_row_group_bytes(None)
        .set_key_value_metadata(Some(
            metadata
                .iter()
                .map(|(key, value)| KeyValue::new(key.to_string(), value.clone()))
                .collect(),
        ))
        .build();
    let options = ArrowWriterOptions::new()
        .with_properties(properties)
        .with_skip_arrow_metadata(true);
    let mut writer =
        ArrowWriter::try_new_with_options(Vec::new(), Arc::clone(&schema), options).map_err(unwritable)?;
    for columns in row_groups {
        let arrays = columns.iter().map(|column| array_of(&column.values)).collect();
        let batch = RecordBatch::try_new(Arc::clone(&schema), arrays).map_err(unwritable)?;
        writer.write(&batch).map_err(unwritable)?;
        writer.flush().map_err(unwritable)?;
    }
    writer.into_inner().map_err(unwritable)
}

/// Every row of a Parquet file, its row groups joined, as named columns.
pub(crate) fn read(bytes: &[u8]) -> Result<Vec<Column>, TableError> {
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(Bytes::copy_from_slice(bytes)).map_err(unreadable)?;
    let fields: Vec<(String, DataType, bool)> = builder
        .schema()
        .fields()
        .iter()
        .map(|field| {
            (
                field.name().clone(),
                field.data_type().clone(),
                field.is_nullable(),
            )
        })
        .collect();
    let mut columns: Vec<Column> = fields
        .iter()
        .map(|(name, kind, nullable)| empty_values(kind, *nullable).map(|values| Column::of(name, values)))
        .collect::<Result<_, _>>()?;
    for batch in builder.build().map_err(unreadable)? {
        let batch = batch.map_err(unreadable)?;
        for (column, array) in columns.iter_mut().zip(batch.columns()) {
            extend(&mut column.values, array.as_ref())?;
        }
    }
    Ok(columns)
}

/// The key-value pairs a Parquet file carries in its footer, in their order.
#[cfg(test)]
pub(crate) fn metadata(bytes: &[u8]) -> Result<Vec<(String, String)>, TableError> {
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(Bytes::copy_from_slice(bytes)).map_err(unreadable)?;
    Ok(builder
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .into_iter()
        .flatten()
        .map(|pair| (pair.key.clone(), pair.value.clone().unwrap_or_default()))
        .collect())
}

fn field_of(column: &Column) -> Field {
    let (kind, nullable) = match &column.values {
        Values::Integers(_) => (DataType::Int64, false),
        Values::SmallIntegers(_) => (DataType::Int32, false),
        Values::OptionalIntegers(_) => (DataType::Int64, true),
        Values::OptionalReals(_) => (DataType::Float64, true),
        Values::OptionalTexts(_) => (DataType::Utf8, true),
        Values::TextLists(_) => (
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, false))),
            false,
        ),
        Values::OptionalBytes(_) => (DataType::Binary, true),
    };
    Field::new(&column.name, kind, nullable)
}

fn array_of(values: &Values) -> ArrayRef {
    match values {
        Values::Integers(values) => Arc::new(Int64Array::from(values.clone())),
        Values::SmallIntegers(values) => Arc::new(Int32Array::from(values.clone())),
        Values::OptionalIntegers(values) => Arc::new(Int64Array::from(values.clone())),
        Values::OptionalReals(values) => Arc::new(Float64Array::from(values.clone())),
        Values::OptionalTexts(values) => Arc::new(StringArray::from(values.clone())),
        Values::TextLists(lists) => {
            let mut builder =
                ListBuilder::new(StringBuilder::new()).with_field(Field::new("item", DataType::Utf8, false));
            for list in lists {
                for text in list {
                    builder.values().append_value(text);
                }
                builder.append(true);
            }
            Arc::new(builder.finish())
        }
        Values::OptionalBytes(values) => Arc::new(BinaryArray::from(
            values
                .iter()
                .map(Option::as_deref)
                .collect::<Vec<Option<&[u8]>>>(),
        )),
    }
}

fn empty_values(kind: &DataType, nullable: bool) -> Result<Values, TableError> {
    match (kind, nullable) {
        (DataType::Int64, false) => Ok(Values::Integers(Vec::new())),
        (DataType::Int32, false) => Ok(Values::SmallIntegers(Vec::new())),
        (DataType::Int64, true) => Ok(Values::OptionalIntegers(Vec::new())),
        (DataType::Float64, true) => Ok(Values::OptionalReals(Vec::new())),
        (DataType::Utf8, true) => Ok(Values::OptionalTexts(Vec::new())),
        (DataType::List(_), false) => Ok(Values::TextLists(Vec::new())),
        (DataType::Binary, true) => Ok(Values::OptionalBytes(Vec::new())),
        (other, nullable) => Err(unreadable(format!(
            "a column of {other}{} is not one a dataset writes",
            if nullable { " or nothing" } else { "" }
        ))),
    }
}

fn extend(values: &mut Values, array: &dyn Array) -> Result<(), TableError> {
    let unexpected = || {
        unreadable(format!(
            "a column held {} where another kind was expected",
            array.data_type()
        ))
    };
    match values {
        Values::Integers(into) => into.extend(
            array
                .as_primitive_opt::<Int64Type>()
                .ok_or_else(unexpected)?
                .values()
                .iter(),
        ),
        Values::SmallIntegers(into) => into.extend(
            array
                .as_primitive_opt::<Int32Type>()
                .ok_or_else(unexpected)?
                .values()
                .iter(),
        ),
        Values::OptionalIntegers(into) => {
            into.extend(array.as_primitive_opt::<Int64Type>().ok_or_else(unexpected)?)
        }
        Values::OptionalReals(into) => {
            into.extend(array.as_primitive_opt::<Float64Type>().ok_or_else(unexpected)?)
        }
        Values::OptionalTexts(into) => into.extend(
            array
                .as_string_opt::<i32>()
                .ok_or_else(unexpected)?
                .iter()
                .map(|text| text.map(str::to_string)),
        ),
        Values::TextLists(into) => {
            let lists = array.as_list_opt::<i32>().ok_or_else(unexpected)?;
            for list in lists.iter() {
                let texts = list.ok_or_else(unexpected)?;
                let texts = texts.as_string_opt::<i32>().ok_or_else(unexpected)?;
                into.push(
                    texts
                        .iter()
                        .map(|text| text.unwrap_or_default().to_string())
                        .collect(),
                );
            }
        }
        Values::OptionalBytes(into) => into.extend(
            array
                .as_binary_opt::<i32>()
                .ok_or_else(unexpected)?
                .iter()
                .map(|bytes| bytes.map(<[u8]>::to_vec)),
        ),
    }
    Ok(())
}
