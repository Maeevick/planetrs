//! Elements set down on disk as they are built, in sorted runs, and read back merged in the order a
//! map index writes them: the planet's elements weigh many times what memory holds.
//!
//! Workers hand their elements over in batches to one sink, which sorts what it holds into a run
//! every couple of gigabytes, so that the runs stay few. A run is written in chunk files, each removed
//! once read, so that merging the runs takes little more room than the tiles they become.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use planetrs_dataset::map_index::{Element, Grid, Line, Placement, Position, Tags};
use rayon::prelude::*;

/// The bytes before each element in a run: its key, its kind, its id and the length of what follows.
const HEADER_BYTES: usize = 8 + 1 + 8 + 4;

/// Elements set down on disk, waiting to be merged.
pub(super) struct Spill {
    directory: PathBuf,
    grid: Grid,
    run_bytes: usize,
    sink: Mutex<Sink>,
    runs: Mutex<Vec<Run>>,
    names: AtomicUsize,
}

/// What the spill holds before it becomes a run: the elements' bytes end to end, and where each
/// begins.
#[derive(Default)]
struct Sink {
    arena: Vec<u8>,
    records: Vec<Record>,
}

/// One element held: what it is sorted by, and where its bytes lie.
#[derive(Clone, Copy)]
struct Record {
    key: u64,
    kind: u8,
    id: i64,
    start: usize,
    length: usize,
}

/// A run on disk: its chunk files in order.
struct Run {
    chunks: VecDeque<PathBuf>,
}

impl Spill {
    /// A spill writing its runs into `directory`, positions kept on `grid`, a run every `run_bytes`.
    pub(super) fn new(directory: &Path, grid: Grid, run_bytes: usize) -> io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        Ok(Self {
            directory: directory.to_path_buf(),
            grid,
            run_bytes,
            sink: Mutex::new(Sink::default()),
            runs: Mutex::new(Vec::new()),
            names: AtomicUsize::new(0),
        })
    }

    /// Sets down an element at its placement, from any thread.
    pub(super) fn add(&self, placement: &Placement, element: &Element) -> io::Result<()> {
        let mut bytes = Vec::new();
        encode(element, self.grid, &mut bytes);
        let full = {
            let mut sink = self.sink.lock().map_err(|_| poisoned())?;
            let start = sink.arena.len();
            sink.records.push(Record {
                key: placement.key,
                kind: placement.order.0,
                id: placement.order.1,
                start,
                length: bytes.len(),
            });
            sink.arena.extend_from_slice(&bytes);
            (sink.arena.len() >= self.run_bytes).then(|| std::mem::take(&mut *sink))
        };
        match full {
            Some(full) => self.write_run(full),
            None => Ok(()),
        }
    }

    /// Every element set down, in the order of their placements, each with its key, removing the
    /// runs as they are read.
    pub(super) fn merged(self) -> io::Result<impl Iterator<Item = io::Result<(u64, Element)>>> {
        let rest = std::mem::take(&mut *self.sink.lock().map_err(|_| poisoned())?);
        if !rest.records.is_empty() {
            self.write_run(rest)?;
        }
        let runs = std::mem::take(&mut *self.runs.lock().map_err(|_| poisoned())?);
        Merged::of(runs, self.grid)
    }

    /// Sorts what a sink held and writes it as a run, in chunks of an eighth of a run.
    fn write_run(&self, mut sink: Sink) -> io::Result<()> {
        sink.records
            .par_sort_unstable_by_key(|record| (record.key, record.kind, record.id));
        let run = self.names.fetch_add(1, Ordering::Relaxed);
        let chunk_bytes = (self.run_bytes / 8).max(1);
        let mut chunks = VecDeque::new();
        let mut out: Option<(BufWriter<File>, usize)> = None;
        for record in &sink.records {
            if out.as_ref().is_none_or(|(_, written)| *written >= chunk_bytes) {
                if let Some((mut full, _)) = out.take() {
                    full.flush()?;
                }
                let path = self.directory.join(format!("run-{run}-{}.bin", chunks.len()));
                out = Some((BufWriter::new(File::create(&path)?), 0));
                chunks.push_back(path);
            }
            if let Some((writer, written)) = out.as_mut() {
                writer.write_all(&record.key.to_le_bytes())?;
                writer.write_all(&[record.kind])?;
                writer.write_all(&record.id.to_le_bytes())?;
                writer.write_all(&(record.length as u32).to_le_bytes())?;
                writer.write_all(&sink.arena[record.start..record.start + record.length])?;
                *written += HEADER_BYTES + record.length;
            }
        }
        if let Some((mut last, _)) = out {
            last.flush()?;
        }
        self.runs.lock().map_err(|_| poisoned())?.push(Run { chunks });
        Ok(())
    }
}

fn poisoned() -> io::Error {
    io::Error::other("a worker failed while setting elements down")
}

/// The runs read together, the next element always the least of their heads.
struct Merged {
    readers: Vec<RunReader>,
    heads: Vec<Vec<u8>>,
    heap: BinaryHeap<Reverse<(u64, u8, i64, usize)>>,
    grid: Grid,
}

impl Merged {
    fn of(runs: Vec<Run>, grid: Grid) -> io::Result<Self> {
        let mut merged = Self {
            readers: runs
                .into_iter()
                .map(|run| RunReader {
                    chunks: run.chunks,
                    current: None,
                })
                .collect(),
            heads: Vec::new(),
            heap: BinaryHeap::new(),
            grid,
        };
        merged.heads = vec![Vec::new(); merged.readers.len()];
        for run in 0..merged.readers.len() {
            merged.advance(run)?;
        }
        Ok(merged)
    }

    /// Reads the next element of `run` into its head.
    fn advance(&mut self, run: usize) -> io::Result<()> {
        if let Some(next) = self.readers[run].next_record()? {
            self.heads[run] = next.bytes;
            self.heap.push(Reverse((next.key, next.kind, next.id, run)));
        }
        Ok(())
    }

    fn next_element(&mut self) -> io::Result<Option<(u64, Element)>> {
        let Some(Reverse((key, _, _, run))) = self.heap.pop() else {
            return Ok(None);
        };
        let bytes = std::mem::take(&mut self.heads[run]);
        self.advance(run)?;
        Ok(Some((key, decode(&bytes, self.grid)?)))
    }
}

impl Iterator for Merged {
    type Item = io::Result<(u64, Element)>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_element().transpose()
    }
}

/// One element read from a run: what it is sorted by, and its bytes.
struct Head {
    key: u64,
    kind: u8,
    id: i64,
    bytes: Vec<u8>,
}

/// A run read a chunk at a time, each chunk removed once read.
struct RunReader {
    chunks: VecDeque<PathBuf>,
    current: Option<(PathBuf, BufReader<File>)>,
}

impl RunReader {
    fn next_record(&mut self) -> io::Result<Option<Head>> {
        loop {
            if self.current.is_none() {
                let Some(path) = self.chunks.pop_front() else {
                    return Ok(None);
                };
                let reader = BufReader::new(File::open(&path)?);
                self.current = Some((path, reader));
            }
            let Some((path, reader)) = self.current.as_mut() else {
                return Ok(None);
            };
            let mut header = [0u8; HEADER_BYTES];
            match read_whole_or_nothing(reader, &mut header)? {
                false => {
                    std::fs::remove_file(&*path)?;
                    self.current = None;
                }
                true => {
                    let key = u64::from_le_bytes(header[0..8].try_into().map_err(|_| not_as_written())?);
                    let kind = header[8];
                    let id = i64::from_le_bytes(header[9..17].try_into().map_err(|_| not_as_written())?);
                    let length = u32::from_le_bytes(header[17..21].try_into().map_err(|_| not_as_written())?);
                    let mut bytes = vec![0u8; length as usize];
                    reader.read_exact(&mut bytes)?;
                    return Ok(Some(Head { key, kind, id, bytes }));
                }
            }
        }
    }
}

/// Fills `buffer` from `reader`, or reads nothing at its end: whether a whole buffer came.
fn read_whole_or_nothing(reader: &mut impl Read, buffer: &mut [u8]) -> io::Result<bool> {
    let mut read = 0;
    while read < buffer.len() {
        match reader.read(&mut buffer[read..])? {
            0 if read == 0 => return Ok(false),
            0 => return Err(not_as_written()),
            more => read += more,
        }
    }
    Ok(true)
}

fn not_as_written() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "an element set down is not as written",
    )
}

/// An element's bytes in a run: its kind and its id, then the rest, positions on `grid` as
/// differences from the one before, every number in as few bytes as it needs. The element says what
/// it is itself, whatever order it is sorted in.
fn encode(element: &Element, grid: Grid, out: &mut Vec<u8>) {
    let (kind, id) = match element {
        Element::Node { id, .. } => (0, *id),
        Element::Way(line) => (1, line.id),
        Element::Multipolygon { id, .. } => (2, *id),
    };
    out.push(kind);
    put_signed(out, id);
    match element {
        Element::Node { at, tags, .. } => {
            let (latitude, longitude) = units_of(*at, grid);
            put_signed(out, latitude);
            put_signed(out, longitude);
            put_tags(out, tags);
        }
        Element::Way(line) => put_line(out, line, grid),
        Element::Multipolygon {
            tags, members, ways, ..
        } => {
            put_tags(out, tags);
            put_number(out, members.len() as u64);
            let mut before = 0;
            for (way, role) in members {
                put_signed(out, way - before);
                before = *way;
                put_text(out, role);
            }
            put_number(out, ways.len() as u64);
            for line in ways {
                put_signed(out, line.id);
                put_line(out, line, grid);
            }
        }
    }
}

/// The element `bytes` stand for.
fn decode(mut bytes: &[u8], grid: Grid) -> io::Result<Element> {
    let bytes = &mut bytes;
    let kind = take(bytes, 1)?[0];
    let id = get_signed(bytes)?;
    match kind {
        0 => {
            let latitude = get_signed(bytes)?;
            let longitude = get_signed(bytes)?;
            Ok(Element::Node {
                id,
                at: position_of(latitude, longitude, grid),
                tags: get_tags(bytes)?,
            })
        }
        1 => Ok(Element::Way(get_line(bytes, id, grid)?)),
        2 => {
            let tags = get_tags(bytes)?;
            let count = get_number(bytes)?;
            let mut members = Vec::new();
            let mut before = 0;
            for _ in 0..count {
                let way = before + get_signed(bytes)?;
                before = way;
                members.push((way, get_text(bytes)?));
            }
            let count = get_number(bytes)?;
            let mut ways = Vec::new();
            for _ in 0..count {
                let way = get_signed(bytes)?;
                ways.push(get_line(bytes, way, grid)?);
            }
            Ok(Element::Multipolygon {
                id,
                tags,
                members,
                ways,
            })
        }
        _ => Err(not_as_written()),
    }
}

fn put_line(out: &mut Vec<u8>, line: &Line, grid: Grid) {
    put_tags(out, &line.tags);
    put_number(out, line.refs.len() as u64);
    let mut before = 0;
    for node in &line.refs {
        put_signed(out, node - before);
        before = *node;
    }
    for eight in line.positions.chunks(8) {
        out.push(
            eight
                .iter()
                .enumerate()
                .filter(|(_, at)| at.is_some())
                .fold(0u8, |byte, (bit, _)| byte | 1 << bit),
        );
    }
    let (mut latitude_before, mut longitude_before) = (0, 0);
    for at in line.positions.iter().flatten() {
        let (latitude, longitude) = units_of(*at, grid);
        put_signed(out, latitude - latitude_before);
        put_signed(out, longitude - longitude_before);
        (latitude_before, longitude_before) = (latitude, longitude);
    }
}

fn get_line(bytes: &mut &[u8], id: i64, grid: Grid) -> io::Result<Line> {
    let tags = get_tags(bytes)?;
    let count = get_number(bytes)? as usize;
    let mut refs = Vec::with_capacity(count);
    let mut before = 0;
    for _ in 0..count {
        let node = before + get_signed(bytes)?;
        before = node;
        refs.push(node);
    }
    let mut placed = Vec::with_capacity(count);
    for start in (0..count).step_by(8) {
        let byte = take(bytes, 1)?[0];
        placed.extend((0..(count - start).min(8)).map(|bit| byte & 1 << bit != 0));
    }
    let (mut latitude, mut longitude) = (0, 0);
    let mut positions = Vec::with_capacity(count);
    for is_placed in placed {
        positions.push(if is_placed {
            latitude += get_signed(bytes)?;
            longitude += get_signed(bytes)?;
            Some(position_of(latitude, longitude, grid))
        } else {
            None
        });
    }
    Ok(Line {
        id,
        tags,
        refs,
        positions,
    })
}

/// A position in the planet's units on `grid`, from which it came exactly.
fn units_of(at: Position, grid: Grid) -> (i64, i64) {
    (
        (at.latitude - grid.latitude_offset) / grid.granularity,
        (at.longitude - grid.longitude_offset) / grid.granularity,
    )
}

fn position_of(latitude: i64, longitude: i64, grid: Grid) -> Position {
    Position {
        latitude: latitude * grid.granularity + grid.latitude_offset,
        longitude: longitude * grid.granularity + grid.longitude_offset,
    }
}

fn put_tags(out: &mut Vec<u8>, tags: &Tags) {
    put_number(out, tags.len() as u64);
    for (key, value) in tags {
        put_text(out, key);
        put_text(out, value);
    }
}

fn get_tags(bytes: &mut &[u8]) -> io::Result<Tags> {
    let count = get_number(bytes)?;
    (0..count)
        .map(|_| Ok((get_text(bytes)?, get_text(bytes)?)))
        .collect()
}

fn put_text(out: &mut Vec<u8>, text: &str) {
    put_number(out, text.len() as u64);
    out.extend_from_slice(text.as_bytes());
}

fn get_text(bytes: &mut &[u8]) -> io::Result<String> {
    let length = get_number(bytes)? as usize;
    String::from_utf8(take(bytes, length)?.to_vec()).map_err(|_| not_as_written())
}

/// A number in seven bits a byte, the high bit saying another byte follows.
fn put_number(out: &mut Vec<u8>, mut number: u64) {
    while number >= 0x80 {
        out.push(number as u8 | 0x80);
        number >>= 7;
    }
    out.push(number as u8);
}

fn get_number(bytes: &mut &[u8]) -> io::Result<u64> {
    let mut number = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = take(bytes, 1)?[0];
        number |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(number);
        }
    }
    Err(not_as_written())
}

/// A signed number folded so that small ones, either side of zero, take few bytes.
fn put_signed(out: &mut Vec<u8>, number: i64) {
    put_number(out, ((number << 1) ^ (number >> 63)) as u64);
}

fn get_signed(bytes: &mut &[u8]) -> io::Result<i64> {
    let folded = get_number(bytes)?;
    Ok((folded >> 1) as i64 ^ -((folded & 1) as i64))
}

fn take<'a>(bytes: &mut &'a [u8], length: usize) -> io::Result<&'a [u8]> {
    if bytes.len() < length {
        return Err(not_as_written());
    }
    let (taken, rest) = bytes.split_at(length);
    *bytes = rest;
    Ok(taken)
}

#[cfg(test)]
mod tests {
    use planetrs_dataset::map_index::{Line, Position};

    use super::*;

    fn grid() -> Grid {
        Grid {
            granularity: 100,
            latitude_offset: 0,
            longitude_offset: 0,
        }
    }

    fn at(latitude: i64, longitude: i64) -> Position {
        Position { latitude, longitude }
    }

    fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).into(), (*value).into()))
            .collect()
    }

    /// Monaco's harbour and Abidjan's port: named and lone nodes, a quay leaving through a node the
    /// planet does not place, and a basin whose inner member lies far away.
    fn harbours() -> Vec<Element> {
        let quay = Line {
            id: 10,
            tags: tags(&[("man_made", "quay"), ("name", "Quai Antoine Ier")]),
            refs: vec![1, 2, 99],
            positions: vec![
                Some(at(43_734_700_000, 7_418_600_000)),
                Some(at(43_735_200_000, 7_421_400_000)),
                None,
            ],
        };
        let island = Line {
            id: 12,
            tags: Vec::new(),
            refs: vec![8, 9, 8],
            positions: vec![
                Some(at(43_800_000_000, 7_500_000_000)),
                Some(at(43_810_000_000, 7_510_000_000)),
                Some(at(43_800_000_000, 7_500_000_000)),
            ],
        };
        vec![
            Element::Node {
                id: 3,
                at: at(43_735_000_000, 7_420_000_000),
                tags: tags(&[("name", "Place d'Armes"), ("place", "square")]),
            },
            Element::Node {
                id: 4,
                at: at(43_736_000_000, 7_419_000_000),
                tags: Vec::new(),
            },
            Element::Way(quay.clone()),
            Element::Multipolygon {
                id: 100,
                tags: tags(&[("type", "multipolygon"), ("natural", "water")]),
                members: vec![(10, "outer".into()), (12, "inner".into())],
                ways: vec![quay, island],
            },
            Element::Way(Line {
                id: 20,
                tags: tags(&[("man_made", "quay")]),
                refs: vec![21, 22],
                positions: vec![
                    Some(at(5_269_000_000, -4_012_000_000)),
                    Some(at(5_270_000_000, -4_010_000_000)),
                ],
            }),
            Element::Node {
                id: 23,
                at: at(-5_269_000_000, -180_000_000_000),
                tags: tags(&[("name", "Île Dérivée")]),
            },
        ]
    }

    #[test]
    fn given_elements_set_down_out_of_order_across_runs_when_merged_then_they_come_back_whole_in_the_order_of_their_placements_and_the_runs_are_gone()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory =
            std::env::temp_dir().join(format!("planetrs-mapclipper-spill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let elements = harbours();
        let spill = Spill::new(&directory, grid(), 200)?;
        for element in elements.iter().rev() {
            spill.add(&element.placement()?, element)?;
        }

        let merged = spill.merged()?.collect::<io::Result<Vec<_>>>()?;

        let mut expected = elements
            .iter()
            .map(|element| element.placement().map(|placement| (placement, element.clone())))
            .collect::<Result<Vec<_>, _>>()?;
        expected.sort_by_key(|(placement, _)| (placement.key, placement.order));
        assert_eq!(
            merged,
            expected
                .into_iter()
                .map(|(placement, element)| (placement.key, element))
                .collect::<Vec<_>>()
        );
        assert_eq!(std::fs::read_dir(&directory)?.count(), 0, "the runs are gone");
        Ok(())
    }
}
