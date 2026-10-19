//! Tables of plain values in and out of Parquet, the one place the `parquet` and `arrow` crates are
//! named, so that swapping them touches nothing else.
//!
//! A table is a list of row groups, each a list of named columns of equal length. The caller cuts
//! the row groups, because only the caller knows where a commune or a multipolygon ends, and every
//! setting that shapes the bytes is pinned here, so that the same table always writes the same file.

use std::ops::Range;
use std::sync::Arc;

use arrow_array::builder::{Int32Builder, Int64Builder, ListBuilder, StringBuilder};
use arrow_array::cast::AsArray;
use arrow_array::types::{Float64Type, Int32Type, Int64Type};
use arrow_array::{
    Array, ArrayRef, BinaryArray, Float64Array, Int32Array, Int64Array, RecordBatch, StringArray, StructArray,
};
use arrow_schema::{DataType, Field, Fields, Schema};
use bytes::{Buf, Bytes};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::{
    ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder,
};
use parquet::arrow::arrow_writer::ArrowWriterOptions;
use parquet::basic::{Compression, GzipLevel};
use parquet::errors::ParquetError;
use parquet::file::metadata::{KeyValue, ParquetMetaData, ParquetMetaDataReader};
use parquet::file::properties::{EnabledStatistics, WriterProperties};
use parquet::file::reader::{ChunkReader, Length};
use parquet::file::statistics::Statistics;
use parquet::schema::types::ColumnPath;

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
    /// UTF-8 texts, none missing.
    Texts(Vec<String>),
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
    /// Byte strings, none missing.
    Bytes(Vec<Vec<u8>>),
    /// Lists of 64-bit integers, each list present and possibly empty.
    IntegerLists(Vec<Vec<i64>>),
    /// Lists of 32-bit integers, each list present and possibly empty.
    SmallIntegerLists(Vec<Vec<i32>>),
    /// Boxes as their west, south, east and north edges, none missing: a struct of `xmin`, `ymin`,
    /// `xmax` and `ymax`, the shape GeoParquet's bounding box covering asks for.
    Boxes(Vec<[f64; 4]>),
}

/// The fields of a box, in the order GeoParquet names them.
const BOX_FIELDS: [&str; 4] = ["xmin", "ymin", "xmax", "ymax"];

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

/// The row groups as one Parquet file, with `metadata` among its key-value pairs and statistics
/// on the columns `pruned_by` alone, the ones a reader picks row groups by.
pub(crate) fn write(
    row_groups: &[Vec<Column>],
    metadata: &[(&str, String)],
    pruned_by: &[&str],
) -> Result<Vec<u8>, TableError> {
    let mut writer = TableWriter::new(metadata, pruned_by);
    for columns in row_groups {
        writer.write(columns)?;
    }
    writer.finish()
}

/// A Parquet file written a row group at a time, so that a table larger than memory is never held
/// whole: only the row group being written and the bytes already encoded are.
pub(crate) struct TableWriter {
    metadata: Vec<(String, String)>,
    pruned_by: Vec<String>,
    open: Option<(ArrowWriter<Vec<u8>>, Arc<Schema>)>,
}

impl TableWriter {
    /// A file to come, with `metadata` among its key-value pairs and statistics on the columns
    /// `pruned_by` alone.
    pub(crate) fn new(metadata: &[(&str, String)], pruned_by: &[&str]) -> Self {
        Self {
            metadata: metadata
                .iter()
                .map(|(key, value)| (key.to_string(), value.clone()))
                .collect(),
            pruned_by: pruned_by.iter().map(|name| name.to_string()).collect(),
            open: None,
        }
    }

    /// Writes one row group. The first fixes the columns every later one has.
    pub(crate) fn write(&mut self, columns: &[Column]) -> Result<(), TableError> {
        if self.open.is_none() {
            self.open = Some(self.opened(columns)?);
        }
        let Some((writer, schema)) = self.open.as_mut() else {
            return Err(unwritable("the file could not be opened"));
        };
        let arrays = columns.iter().map(|column| array_of(&column.values)).collect();
        let batch = RecordBatch::try_new(Arc::clone(schema), arrays).map_err(unwritable)?;
        writer.write(&batch).map_err(unwritable)?;
        writer.flush().map_err(unwritable)
    }

    /// The file's bytes, its footer written after the last row group.
    pub(crate) fn finish(self) -> Result<Vec<u8>, TableError> {
        let (writer, _) = match self.open {
            Some(open) => open,
            None => self.opened(&[])?,
        };
        writer.into_inner().map_err(unwritable)
    }

    fn opened(&self, columns: &[Column]) -> Result<(ArrowWriter<Vec<u8>>, Arc<Schema>), TableError> {
        let schema = Arc::new(Schema::new(columns.iter().map(field_of).collect::<Vec<_>>()));
        let properties = WriterProperties::builder()
            .set_created_by(WRITER.to_string())
            .set_compression(Compression::GZIP(
                GzipLevel::try_new(GZIP_LEVEL).map_err(unwritable)?,
            ))
            .set_statistics_enabled(EnabledStatistics::None)
            .set_max_row_group_row_count(None)
            .set_max_row_group_bytes(None)
            .set_key_value_metadata(Some(
                self.metadata
                    .iter()
                    .map(|(key, value)| KeyValue::new(key.clone(), value.clone()))
                    .collect(),
            ));
        let properties = self
            .pruned_by
            .iter()
            .fold(properties, |properties, name| {
                let path = ColumnPath::new(name.split('.').map(str::to_string).collect());
                properties.set_column_statistics_enabled(path, EnabledStatistics::Chunk)
            })
            .build();
        let options = ArrowWriterOptions::new()
            .with_properties(properties)
            .with_skip_arrow_metadata(true);
        let writer = ArrowWriter::try_new_with_options(Vec::new(), Arc::clone(&schema), options)
            .map_err(unwritable)?;
        Ok((writer, schema))
    }
}

/// Every row of a Parquet file, its row groups joined, as named columns.
pub(crate) fn read(bytes: &[u8]) -> Result<Vec<Column>, TableError> {
    columns_from(ParquetRecordBatchReaderBuilder::try_new(Bytes::copy_from_slice(bytes)).map_err(unreadable)?)
}

/// The columns a reader yields, its batches joined.
fn columns_from<T: ChunkReader + 'static>(
    builder: ParquetRecordBatchReaderBuilder<T>,
) -> Result<Vec<Column>, TableError> {
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

/// One row group as its footer describes it: how many rows, and the lowest and highest value of an
/// integer column, which is what a reader picks row groups by.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowGroupSpan {
    pub(crate) rows: i64,
    pub(crate) lowest: i64,
    pub(crate) highest: i64,
}

/// Where each row group of a file lies, with the lowest value of the integer column `name`.
#[cfg(test)]
pub(crate) fn row_group_bytes(bytes: &[u8], name: &str) -> Result<Vec<(i64, Range<u64>)>, TableError> {
    match footer_from_tail(bytes, bytes.len() as u64)? {
        Tail::Footer(footer) => Ok(footer
            .row_groups(name)
            .into_iter()
            .map(|group| (group.lowest.unwrap_or_default(), group.bytes))
            .collect()),
        Tail::Short { .. } => Err(unreadable("a whole file is shorter than its footer")),
    }
}

/// The columns of a file whose row groups carry statistics, in schema order.
#[cfg(test)]
pub(crate) fn columns_with_statistics(bytes: &[u8]) -> Result<Vec<String>, TableError> {
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(Bytes::copy_from_slice(bytes)).map_err(unreadable)?;
    let metadata = builder.metadata();
    let names: Vec<String> = metadata
        .file_metadata()
        .schema_descr()
        .columns()
        .iter()
        .map(|column| column.path().string())
        .collect();
    Ok(names
        .into_iter()
        .enumerate()
        .filter(|(at, _)| {
            metadata
                .row_groups()
                .iter()
                .any(|group| group.column(*at).statistics().is_some())
        })
        .map(|(_, name)| name)
        .collect())
}

/// Every row group of a file, with the span of the integer column `name`.
#[cfg(test)]
pub(crate) fn row_groups(bytes: &[u8], name: &str) -> Result<Vec<RowGroupSpan>, TableError> {
    use parquet::file::statistics::Statistics;
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(Bytes::copy_from_slice(bytes)).map_err(unreadable)?;
    let metadata = builder.metadata();
    let column = metadata
        .file_metadata()
        .schema_descr()
        .columns()
        .iter()
        .position(|column| column.name() == name)
        .ok_or_else(|| kind_mismatch(name))?;
    metadata
        .row_groups()
        .iter()
        .map(|group| match group.column(column).statistics() {
            Some(Statistics::Int64(span)) => match (span.min_opt(), span.max_opt()) {
                (Some(lowest), Some(highest)) => Ok(RowGroupSpan {
                    rows: group.num_rows(),
                    lowest: *lowest,
                    highest: *highest,
                }),
                _ => Err(kind_mismatch(name)),
            },
            _ => Err(kind_mismatch(name)),
        })
        .collect()
}

/// A file's footer, read from its end alone: what a reader needs to choose the row groups to fetch.
#[derive(Debug, Clone)]
pub(crate) struct Footer {
    metadata: Arc<ParquetMetaData>,
    length: u64,
}

/// What a file's last bytes gave: its footer, or how many bytes from its end the footer needs.
#[derive(Debug, Clone)]
pub(crate) enum Tail {
    Footer(Footer),
    Short { needed: u64 },
}

/// One row group as a reader chooses it: where its bytes lie, and the lowest and highest value of
/// the integer column it is pruned by, when the footer carries them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RowGroupBytes {
    pub(crate) index: usize,
    pub(crate) bytes: Range<u64>,
    pub(crate) lowest: Option<i64>,
    pub(crate) highest: Option<i64>,
}

/// The footer of a file of `length` bytes, from its last bytes `tail`, or how many it needs.
pub(crate) fn footer_from_tail(tail: &[u8], length: u64) -> Result<Tail, TableError> {
    let mut reader = ParquetMetaDataReader::new();
    match reader.try_parse_sized(&Bytes::copy_from_slice(tail), length) {
        Ok(()) => Ok(Tail::Footer(Footer {
            metadata: Arc::new(reader.finish().map_err(unreadable)?),
            length,
        })),
        Err(ParquetError::NeedMoreData(needed)) => Ok(Tail::Short {
            needed: needed as u64,
        }),
        Err(other) => Err(unreadable(other)),
    }
}

impl Footer {
    /// The value the footer carries under `key`, if it carries one.
    pub(crate) fn metadata_value(&self, key: &str) -> Option<String> {
        self.metadata
            .file_metadata()
            .key_value_metadata()?
            .iter()
            .find(|pair| pair.key == key)
            .and_then(|pair| pair.value.clone())
    }

    /// Every row group, with the span of the integer column `pruned_by`.
    pub(crate) fn row_groups(&self, pruned_by: &str) -> Vec<RowGroupBytes> {
        let column = self
            .metadata
            .file_metadata()
            .schema_descr()
            .columns()
            .iter()
            .position(|column| column.name() == pruned_by);
        self.metadata
            .row_groups()
            .iter()
            .enumerate()
            .map(|(index, group)| {
                let (start, end) = group.columns().iter().fold((u64::MAX, 0), |(start, end), chunk| {
                    let (offset, length) = chunk.byte_range();
                    (start.min(offset), end.max(offset + length))
                });
                let span = column.and_then(|column| match group.column(column).statistics() {
                    Some(Statistics::Int64(span)) => Some((span.min_opt().copied(), span.max_opt().copied())),
                    _ => None,
                });
                RowGroupBytes {
                    index,
                    bytes: start..end,
                    lowest: span.and_then(|(lowest, _)| lowest),
                    highest: span.and_then(|(_, highest)| highest),
                }
            })
            .collect()
    }

    /// Every row group, with the box around the boxes of its rows in the struct column `name`, when
    /// the footer carries their statistics: the lowest `xmin` and `ymin` and the highest `xmax` and
    /// `ymax`, which is what a reader picks the row groups near a box by.
    pub(crate) fn row_group_bounds(&self, name: &str) -> Vec<(RowGroupBytes, Option<[f64; 4]>)> {
        let columns = self.metadata.file_metadata().schema_descr().columns();
        let leaves: Vec<Option<usize>> = BOX_FIELDS
            .iter()
            .map(|field| {
                columns
                    .iter()
                    .position(|column| column.path().parts() == [name.to_string(), field.to_string()])
            })
            .collect();
        self.row_groups(name)
            .into_iter()
            .map(|group| {
                let row_group = self.metadata.row_group(group.index);
                let edge = |at: usize, lowest: bool| match row_group.column(leaves[at]?).statistics() {
                    Some(Statistics::Double(span)) if lowest => span.min_opt().copied(),
                    Some(Statistics::Double(span)) => span.max_opt().copied(),
                    _ => None,
                };
                let bounds = (|| Some([edge(0, true)?, edge(1, true)?, edge(2, false)?, edge(3, false)?]))();
                (group, bounds)
            })
            .collect()
    }

    /// The columns of one row group, decoded from the bytes of its range alone.
    pub(crate) fn read_row_group(
        &self,
        group: &RowGroupBytes,
        bytes: Vec<u8>,
    ) -> Result<Vec<Column>, TableError> {
        let held = SparseFile {
            start: group.bytes.start,
            bytes: Bytes::from(bytes),
            length: self.length,
        };
        let metadata = ArrowReaderMetadata::try_new(Arc::clone(&self.metadata), ArrowReaderOptions::new())
            .map_err(unreadable)?;
        let builder = ParquetRecordBatchReaderBuilder::new_with_metadata(held, metadata)
            .with_row_groups(vec![group.index]);
        columns_from(builder)
    }
}

/// The bytes of one range of a file, standing in for the whole file to a reader that asks only
/// for that range.
struct SparseFile {
    start: u64,
    bytes: Bytes,
    length: u64,
}

impl SparseFile {
    fn slice(&self, start: u64, length: usize) -> Result<Bytes, ParquetError> {
        let from = start
            .checked_sub(self.start)
            .map(|from| from as usize)
            .filter(|from| from + length <= self.bytes.len())
            .ok_or_else(|| {
                ParquetError::General(format!(
                    "bytes {start}..{} were not fetched",
                    start + length as u64
                ))
            })?;
        Ok(self.bytes.slice(from..from + length))
    }
}

impl Length for SparseFile {
    fn len(&self) -> u64 {
        self.length
    }
}

impl ChunkReader for SparseFile {
    type T = bytes::buf::Reader<Bytes>;

    fn get_read(&self, start: u64) -> Result<Self::T, ParquetError> {
        let rest = (self.start + self.bytes.len() as u64).saturating_sub(start) as usize;
        Ok(self.slice(start, rest)?.reader())
    }

    fn get_bytes(&self, start: u64, length: usize) -> Result<Bytes, ParquetError> {
        self.slice(start, length)
    }
}

fn field_of(column: &Column) -> Field {
    let (kind, nullable) = match &column.values {
        Values::Integers(_) => (DataType::Int64, false),
        Values::SmallIntegers(_) => (DataType::Int32, false),
        Values::Texts(_) => (DataType::Utf8, false),
        Values::OptionalIntegers(_) => (DataType::Int64, true),
        Values::OptionalReals(_) => (DataType::Float64, true),
        Values::OptionalTexts(_) => (DataType::Utf8, true),
        Values::TextLists(_) => (
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, false))),
            false,
        ),
        Values::OptionalBytes(_) => (DataType::Binary, true),
        Values::Bytes(_) => (DataType::Binary, false),
        Values::IntegerLists(_) => (
            DataType::List(Arc::new(Field::new("item", DataType::Int64, false))),
            false,
        ),
        Values::SmallIntegerLists(_) => (
            DataType::List(Arc::new(Field::new("item", DataType::Int32, false))),
            false,
        ),
        Values::Boxes(_) => (DataType::Struct(box_fields()), false),
    };
    Field::new(&column.name, kind, nullable)
}

fn box_fields() -> Fields {
    BOX_FIELDS
        .iter()
        .map(|name| Field::new(*name, DataType::Float64, false))
        .collect()
}

fn array_of(values: &Values) -> ArrayRef {
    match values {
        Values::Integers(values) => Arc::new(Int64Array::from(values.clone())),
        Values::SmallIntegers(values) => Arc::new(Int32Array::from(values.clone())),
        Values::Texts(values) => Arc::new(StringArray::from(values.clone())),
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
        Values::Bytes(values) => Arc::new(BinaryArray::from(
            values.iter().map(Vec::as_slice).collect::<Vec<&[u8]>>(),
        )),
        Values::IntegerLists(lists) => {
            let mut builder =
                ListBuilder::new(Int64Builder::new()).with_field(Field::new("item", DataType::Int64, false));
            for list in lists {
                builder.values().append_slice(list);
                builder.append(true);
            }
            Arc::new(builder.finish())
        }
        Values::SmallIntegerLists(lists) => {
            let mut builder =
                ListBuilder::new(Int32Builder::new()).with_field(Field::new("item", DataType::Int32, false));
            for list in lists {
                builder.values().append_slice(list);
                builder.append(true);
            }
            Arc::new(builder.finish())
        }
        Values::Boxes(boxes) => {
            let edges: Vec<ArrayRef> = (0..BOX_FIELDS.len())
                .map(|edge| {
                    Arc::new(Float64Array::from_iter_values(
                        boxes.iter().map(|box_| box_[edge]),
                    )) as ArrayRef
                })
                .collect();
            Arc::new(StructArray::new(box_fields(), edges, None))
        }
    }
}

fn empty_values(kind: &DataType, nullable: bool) -> Result<Values, TableError> {
    match (kind, nullable) {
        (DataType::Int64, false) => Ok(Values::Integers(Vec::new())),
        (DataType::Int32, false) => Ok(Values::SmallIntegers(Vec::new())),
        (DataType::Utf8, false) => Ok(Values::Texts(Vec::new())),
        (DataType::Int64, true) => Ok(Values::OptionalIntegers(Vec::new())),
        (DataType::Float64, true) => Ok(Values::OptionalReals(Vec::new())),
        (DataType::Utf8, true) => Ok(Values::OptionalTexts(Vec::new())),
        (DataType::List(item), false) => match item.data_type() {
            DataType::Utf8 => Ok(Values::TextLists(Vec::new())),
            DataType::Int64 => Ok(Values::IntegerLists(Vec::new())),
            DataType::Int32 => Ok(Values::SmallIntegerLists(Vec::new())),
            other => Err(unreadable(format!(
                "a list of {other} is not one a dataset writes"
            ))),
        },
        (DataType::Binary, true) => Ok(Values::OptionalBytes(Vec::new())),
        (DataType::Binary, false) => Ok(Values::Bytes(Vec::new())),
        (DataType::Struct(fields), false)
            if fields.iter().map(|field| field.name().as_str()).eq(BOX_FIELDS) =>
        {
            Ok(Values::Boxes(Vec::new()))
        }
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
        Values::Texts(into) => into.extend(
            array
                .as_string_opt::<i32>()
                .ok_or_else(unexpected)?
                .iter()
                .map(|text| text.unwrap_or_default().to_string()),
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
        Values::Bytes(into) => into.extend(
            array
                .as_binary_opt::<i32>()
                .ok_or_else(unexpected)?
                .iter()
                .map(|bytes| bytes.unwrap_or_default().to_vec()),
        ),
        Values::IntegerLists(into) => {
            for list in array.as_list_opt::<i32>().ok_or_else(unexpected)?.iter() {
                let list = list.ok_or_else(unexpected)?;
                into.push(
                    list.as_primitive_opt::<Int64Type>()
                        .ok_or_else(unexpected)?
                        .values()
                        .to_vec(),
                );
            }
        }
        Values::SmallIntegerLists(into) => {
            for list in array.as_list_opt::<i32>().ok_or_else(unexpected)?.iter() {
                let list = list.ok_or_else(unexpected)?;
                into.push(
                    list.as_primitive_opt::<Int32Type>()
                        .ok_or_else(unexpected)?
                        .values()
                        .to_vec(),
                );
            }
        }
        Values::Boxes(into) => {
            let boxes = array.as_struct_opt().ok_or_else(unexpected)?;
            let edges: Vec<&Float64Array> = (0..BOX_FIELDS.len())
                .map(|edge| {
                    boxes
                        .column(edge)
                        .as_primitive_opt::<Float64Type>()
                        .ok_or_else(unexpected)
                })
                .collect::<Result<_, _>>()?;
            into.extend((0..boxes.len()).map(|row| {
                [
                    edges[0].value(row),
                    edges[1].value(row),
                    edges[2].value(row),
                    edges[3].value(row),
                ]
            }));
        }
    }
    Ok(())
}

fn kind_mismatch(name: &str) -> TableError {
    unreadable(format!("the column {name} is missing or of another kind"))
}

fn named<'a>(columns: &'a [Column], name: &str) -> Result<&'a Values, TableError> {
    columns
        .iter()
        .find(|column| column.name == name)
        .map(|column| &column.values)
        .ok_or_else(|| kind_mismatch(name))
}

/// The values of the column `name`, when it holds 64-bit integers, none missing.
pub(crate) fn integers<'a>(columns: &'a [Column], name: &str) -> Result<&'a [i64], TableError> {
    match named(columns, name)? {
        Values::Integers(values) => Ok(values),
        _ => Err(kind_mismatch(name)),
    }
}

/// The values of the column `name`, when it holds 32-bit integers, none missing.
pub(crate) fn small_integers<'a>(columns: &'a [Column], name: &str) -> Result<&'a [i32], TableError> {
    match named(columns, name)? {
        Values::SmallIntegers(values) => Ok(values),
        _ => Err(kind_mismatch(name)),
    }
}

/// The values of the column `name`, when it holds texts, none missing.
pub(crate) fn texts<'a>(columns: &'a [Column], name: &str) -> Result<&'a [String], TableError> {
    match named(columns, name)? {
        Values::Texts(values) => Ok(values),
        _ => Err(kind_mismatch(name)),
    }
}

/// The values of the column `name`, when it holds 64-bit integers, some missing.
pub(crate) fn optional_integers<'a>(
    columns: &'a [Column],
    name: &str,
) -> Result<&'a [Option<i64>], TableError> {
    match named(columns, name)? {
        Values::OptionalIntegers(values) => Ok(values),
        _ => Err(kind_mismatch(name)),
    }
}

/// The values of the column `name`, when it holds doubles, some missing.
pub(crate) fn optional_reals<'a>(columns: &'a [Column], name: &str) -> Result<&'a [Option<f64>], TableError> {
    match named(columns, name)? {
        Values::OptionalReals(values) => Ok(values),
        _ => Err(kind_mismatch(name)),
    }
}

/// The values of the column `name`, when it holds texts, some missing.
pub(crate) fn optional_texts<'a>(
    columns: &'a [Column],
    name: &str,
) -> Result<&'a [Option<String>], TableError> {
    match named(columns, name)? {
        Values::OptionalTexts(values) => Ok(values),
        _ => Err(kind_mismatch(name)),
    }
}

/// The values of the column `name`, when it holds lists of texts.
pub(crate) fn text_lists<'a>(columns: &'a [Column], name: &str) -> Result<&'a [Vec<String>], TableError> {
    match named(columns, name)? {
        Values::TextLists(values) => Ok(values),
        _ => Err(kind_mismatch(name)),
    }
}

/// The values of the column `name`, when it holds byte strings, some missing.
pub(crate) fn optional_bytes<'a>(
    columns: &'a [Column],
    name: &str,
) -> Result<&'a [Option<Vec<u8>>], TableError> {
    match named(columns, name)? {
        Values::OptionalBytes(values) => Ok(values),
        _ => Err(kind_mismatch(name)),
    }
}

/// The values of the column `name`, when it holds byte strings, none missing.
pub(crate) fn bytes<'a>(columns: &'a [Column], name: &str) -> Result<&'a [Vec<u8>], TableError> {
    match named(columns, name)? {
        Values::Bytes(values) => Ok(values),
        _ => Err(kind_mismatch(name)),
    }
}

/// The values of the column `name`, when it holds lists of 64-bit integers.
pub(crate) fn integer_lists<'a>(columns: &'a [Column], name: &str) -> Result<&'a [Vec<i64>], TableError> {
    match named(columns, name)? {
        Values::IntegerLists(values) => Ok(values),
        _ => Err(kind_mismatch(name)),
    }
}

/// The values of the column `name`, when it holds lists of 32-bit integers.
pub(crate) fn small_integer_lists<'a>(
    columns: &'a [Column],
    name: &str,
) -> Result<&'a [Vec<i32>], TableError> {
    match named(columns, name)? {
        Values::SmallIntegerLists(values) => Ok(values),
        _ => Err(kind_mismatch(name)),
    }
}

/// The values of the column `name`, when it holds boxes.
pub(crate) fn boxes<'a>(columns: &'a [Column], name: &str) -> Result<&'a [[f64; 4]], TableError> {
    match named(columns, name)? {
        Values::Boxes(values) => Ok(values),
        _ => Err(kind_mismatch(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_small_table() -> Result<Vec<u8>, TableError> {
        write(
            &[vec![Column::of(
                "relation",
                Values::Integers(vec![946_433, 1_402_847]),
            )]],
            &[],
            &["relation"],
        )
    }

    #[test]
    fn given_a_tail_shorter_than_the_footer_when_read_then_it_says_how_many_bytes_from_the_end_are_needed()
    -> Result<(), TableError> {
        let file = a_small_table()?;
        let length = file.len() as u64;

        let read = footer_from_tail(&file[file.len() - 12..], length)?;

        assert!(
            matches!(read, Tail::Short { needed } if needed > 12 && needed <= length),
            "{read:?}"
        );
        Ok(())
    }

    #[test]
    fn given_a_tail_holding_the_whole_footer_when_read_then_its_row_groups_come_with_their_spans()
    -> Result<(), TableError> {
        let file = a_small_table()?;

        let read = footer_from_tail(&file, file.len() as u64)?;

        let Tail::Footer(footer) = read else {
            return Err(unreadable("the footer was not found"));
        };
        let groups = footer.row_groups("relation");
        assert_eq!(
            groups
                .iter()
                .map(|group| (group.index, group.lowest, group.highest))
                .collect::<Vec<_>>(),
            vec![(0, Some(946_433), Some(1_402_847))]
        );
        Ok(())
    }
}
