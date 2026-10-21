//! Filing what the planet holds under the countries asked for, as their map index cut into tiles, so
//! that a map of any box inside them is cut from the dataset rather than from the planet.
//!
//! The map index holds what passes one to five of a harvest would take out of the planet for any box
//! inside the countries: every way with a node in one of them, whole, with every node the planet
//! places; every node of theirs that carries a name or that no way runs through, the others standing
//! in their ways; and every multipolygon with a member among those ways, each member way whole.
//!
//! The build holds little in memory, so that it scales to the whole planet: every node's position goes
//! into a file indexed by id, every element is set down on disk as it is built, and the tiles are
//! written from the elements merged back in the order of their placements.

mod countries;
mod positions;
mod scan;
mod spill;

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use planetrs_dataset::directory::{Written, record_all_written, written_beside};
use planetrs_dataset::map_index::{
    Element, Grid, Line, Placement, Position, Scope, Summary, Tags, Tile, TileWeights, TileWriter,
    TileWritten,
};
use planetrs_dataset::{Part, Provenance};
use planetrs_geo::territory::{Coverage, IdentifierSet};
use planetrs_pbf::{Block, PbfError, Planet, block, dense_group, id_span, visit_relations, visit_ways};

use self::countries::{Countries, countries_in};
use self::positions::NodePositions;
use self::scan::{every_blob, scan};
use self::spill::Spill;
use crate::error::ClipError;
use crate::harvest::tag;

const MULTIPOLYGON: &str = "multipolygon";

/// The finest grid a map index holds positions on: a stored unit of 100 nanodegrees, as every
/// planet published stores them, keeps a position in 32 bits.
const FINEST_GRANULARITY: i64 = 100;

/// How much a spill holds in memory before it sorts it into a run on disk.
const RUN_BYTES: usize = 1 << 31;

/// What a map index holds, once built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapIndexed {
    /// Whom it was built for: the planet, or the countries asked for.
    pub scope: Scope,
    /// Nodes kept for a name or for standing alone.
    pub nodes: usize,
    /// Ways with a node in one of the countries.
    pub ways: usize,
    /// Multipolygons with a member among those ways.
    pub multipolygons: usize,
    /// The tiles they were cut into.
    pub tiles: usize,
}

/// Builds the map index `scope` asks for, of the whole planet or of countries, from the planet file
/// at `planet`, cut into tiles, and records it in the dataset in `directory` in place of any map
/// index it held.
///
/// Its tiles weigh at most `tile_weight` each (see [`planetrs_dataset::map_index::TILE_WEIGHT`]),
/// but where one cell of the finest tiling weighs more. The build's work files go beside the dataset
/// and are removed however it ends. `progress` hears how many blobs each pass has read, as a
/// harvest's caller does.
pub fn build_map_index(
    planet: &Path,
    directory: &Path,
    scope: &Scope,
    tile_weight: u64,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<MapIndexed, ClipError> {
    let path = planet;
    let planet = Planet::open(path)?;
    let source = Provenance::of(path, &planet);
    let sections = planet.sections()?;
    let territory = match scope {
        Scope::Planet => None,
        Scope::Countries(codes) => {
            let codes: Vec<&str> = codes.iter().map(String::as_str).collect();
            Some(countries_in(&planet, &sections, &codes, progress)?)
        }
    };
    let (scope, coverage) = match &territory {
        None => (Scope::Planet, Coverage::everywhere()),
        Some(countries) => (
            Scope::Countries(countries.codes().map(str::to_string).collect()),
            countries.coverage(),
        ),
    };
    std::fs::create_dir_all(directory).map_err(|failure| ClipError::writing(directory, failure))?;
    let work = Work::beside(directory).map_err(|failure| ClipError::writing(directory, failure))?;
    let failure = WorkFailure::default();
    let grid = first_grid(&planet, sections.nodes.clone())?;
    let largest_node = largest_id(&planet, sections.nodes.clone())?;
    let largest_way = largest_id(&planet, sections.ways.clone())?;
    let spill = Spill::new(&work.join("elements"), grid, RUN_BYTES).map_err(|e| work.failed(e))?;
    let weights = TileWeights::new();
    let elements = SetDown {
        spill: &spill,
        weights: &weights,
    };

    let mut multipolygons = scan(
        &planet,
        &every_blob(sections.relations.clone()),
        "relations",
        progress,
        Vec::new,
        visit_multipolygons,
        concatenated,
    )?
    .found;
    multipolygons.sort_unstable_by_key(|multipolygon| multipolygon.id);
    let member_ways = IdentifierSet::up_to(largest_way);
    for multipolygon in &multipolygons {
        for (way, _) in &multipolygon.members {
            member_ways.insert(*way);
        }
    }

    let positions =
        NodePositions::create(&work.join("positions"), largest_node).map_err(|e| work.failed(e))?;
    let inside = IdentifierSet::up_to(largest_node);
    let named = IdentifierSet::up_to(largest_node);
    let nodes_kept = AtomicUsize::new(0);
    let grids = scan(
        &planet,
        &every_blob(sections.nodes.clone()),
        "nodes",
        progress,
        GridsSeen::default,
        |seen, decoded| {
            let node_pass = NodePass {
                territory: territory.as_ref(),
                positions: &positions,
                inside: &inside,
                named: &named,
                grid,
                elements: &elements,
                kept: &nodes_kept,
                failure: &failure,
            };
            node_pass.visit(seen, decoded)
        },
        GridsSeen::absorb,
    )
    .map_err(|error| failure.explained(&work, error))?
    .found;
    if grid_of(&grids.grids, grids.out_of_range)? != grid {
        return Err(ClipError::Grid {
            detail: "its node blocks use more than one grid".into(),
        });
    }

    let referenced = IdentifierSet::up_to(largest_node);
    let kept_ways = IdentifierSet::up_to(largest_way);
    let members = Spill::new(&work.join("members"), grid, RUN_BYTES).map_err(|e| work.failed(e))?;
    let ways_kept = AtomicUsize::new(0);
    scan(
        &planet,
        &every_blob(sections.ways.clone()),
        "ways",
        progress,
        || (),
        |(), decoded| {
            let way_pass = WayPass {
                inside: &inside,
                member_ways: &member_ways,
                positions: &positions,
                grid,
                referenced: &referenced,
                kept_ways: &kept_ways,
                elements: &elements,
                members: &members,
                kept: &ways_kept,
                failure: &failure,
            };
            way_pass.visit(decoded)
        },
        |(), ()| (),
    )
    .map_err(|error| failure.explained(&work, error))?;

    positions
        .visit_each(|id, latitude, longitude| {
            if inside.contains(id) && !referenced.contains(id) && !named.contains(id) {
                let lone = Element::Node {
                    id,
                    at: position_on(grid, latitude, longitude),
                    tags: Tags::new(),
                };
                match elements.add(&lone) {
                    Ok(true) => {
                        nodes_kept.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(false) => {}
                    Err(error) => {
                        failure.keep(error);
                    }
                }
            }
        })
        .map_err(|e| work.failed(e))?;
    failure.raise(&work)?;
    drop(positions);

    let multipolygons_kept =
        assemble_multipolygons(&multipolygons, &kept_ways, members, &work, grid, &elements)?;

    let tiles = weights.tiles(tile_weight);
    let mut written = write_tiles(tiles, spill, directory, &work, grid)?;
    let tiles = written.len();
    let summary = Summary {
        scope: scope.clone(),
        grid,
        coverage,
    };
    let beside = written_beside(directory, SUMMARY);
    std::fs::write(&beside, summary.to_json()).map_err(|failure| ClipError::writing(&beside, failure))?;
    written.insert(
        0,
        Written {
            part: Part::MapSummary,
            name: SUMMARY.into(),
            rows: tiles as u64,
            bbox: None,
        },
    );
    record_all_written(directory, &source, is_the_map_index, written).map_err(|failure| {
        ClipError::Recording {
            directory: directory.to_path_buf(),
            source: failure,
        }
    })?;
    Ok(MapIndexed {
        scope,
        nodes: nodes_kept.into_inner(),
        ways: ways_kept.into_inner(),
        multipolygons: multipolygons_kept,
        tiles,
    })
}

/// The name of a map index's summary in a dataset.
const SUMMARY: &str = "map-summary.json";

/// Whether a part of a dataset belongs to the map index, which a build replaces whole.
fn is_the_map_index(part: &Part) -> bool {
    matches!(part, Part::MapSummary | Part::MapTile { .. })
}

/// The build's work files, beside the dataset, removed however the build ends.
struct Work(PathBuf);

impl Work {
    fn beside(directory: &Path) -> io::Result<Self> {
        let path = directory.join("map-index.work");
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// A failure writing or reading the work files, said for what it is.
    fn failed(&self, failure: io::Error) -> ClipError {
        ClipError::writing(&self.0, failure)
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The first failure writing the build's work files: it stops the pass under way, and is said for
/// what it is rather than as a planet that would not read.
#[derive(Default)]
struct WorkFailure(Mutex<Option<io::Error>>);

impl WorkFailure {
    /// Keeps the failure, the first one only, and gives the error that stops the pass.
    fn keep(&self, failure: io::Error) -> PbfError {
        if let Ok(mut first) = self.0.lock() {
            first.get_or_insert(failure);
        }
        PbfError::Io(io::Error::other("the build's work files failed"))
    }

    /// The failure a pass stopped on: the work files' when they failed, else the planet's.
    fn explained(&self, work: &Work, error: PbfError) -> ClipError {
        match self.0.lock().ok().and_then(|mut first| first.take()) {
            Some(failure) => work.failed(failure),
            None => ClipError::Planet(error),
        }
    }

    /// The failure kept, if any.
    fn raise(&self, work: &Work) -> Result<(), ClipError> {
        match self.0.lock().ok().and_then(|mut first| first.take()) {
            Some(failure) => Err(work.failed(failure)),
            None => Ok(()),
        }
    }
}

/// Where elements go as they are built: set down on disk, their weights summed by cell.
struct SetDown<'a> {
    spill: &'a Spill,
    weights: &'a TileWeights,
}

impl SetDown<'_> {
    /// Sets an element down, or leaves it out when the planet places none of its nodes, since it has
    /// no place on a map: whether it was set down.
    fn add(&self, element: &Element) -> io::Result<bool> {
        let Ok(placement) = element.placement() else {
            return Ok(false);
        };
        self.weights.add(placement.key, placement.weight);
        self.spill.add(&placement, element)?;
        Ok(true)
    }
}

/// What the pass over every node saw of the grids its blocks store positions on.
#[derive(Default)]
struct GridsSeen {
    grids: Vec<Grid>,
    out_of_range: bool,
}

impl GridsSeen {
    fn absorb(mut self, other: Self) -> Self {
        for grid in other.grids {
            if !self.grids.contains(&grid) {
                self.grids.push(grid);
            }
        }
        self.out_of_range |= other.out_of_range;
        self
    }
}

/// The pass over every node: each position written to the file, the nodes inside the countries
/// marked, every node when the whole planet is indexed, and their named ones set down with every
/// tag.
struct NodePass<'a> {
    territory: Option<&'a Countries>,
    positions: &'a NodePositions,
    inside: &'a IdentifierSet,
    named: &'a IdentifierSet,
    grid: Grid,
    elements: &'a SetDown<'a>,
    kept: &'a AtomicUsize,
    failure: &'a WorkFailure,
}

impl NodePass<'_> {
    fn visit(
        &self,
        seen: &mut GridsSeen,
        decoded: &Block<'_>,
    ) -> Result<Option<planetrs_pbf::IdSpan>, PbfError> {
        let block_grid = Grid {
            granularity: decoded.granularity,
            latitude_offset: decoded.lat_offset,
            longitude_offset: decoded.lon_offset,
        };
        if !seen.grids.contains(&block_grid) {
            seen.grids.push(block_grid);
        }
        // A block whose strings never mention `name` holds no named node, and is walked without its tags.
        let name_key = decoded.string_id(b"name");
        for group in &decoded.groups {
            let Some(dense) = dense_group(group)? else {
                continue;
            };
            let mut stored: Vec<(i64, i32, i32)> = Vec::new();
            let mut named: Vec<Element> = Vec::new();
            let mut keep = |id: i64, latitude: i64, longitude: i64, tags: Option<&[(u32, u32)]>| {
                let (Ok(latitude), Ok(longitude)) = (i32::try_from(latitude), i32::try_from(longitude))
                else {
                    seen.out_of_range = true;
                    return;
                };
                stored.push((id, latitude, longitude));
                let point = [
                    decoded.longitude(i64::from(longitude)),
                    decoded.latitude(i64::from(latitude)),
                ];
                if !self.territory.is_none_or(|countries| countries.hold(point)) {
                    return;
                }
                self.inside.insert(id);
                let Some(tags) = tags.filter(|tags| tags.iter().any(|(key, _)| Some(*key) == name_key))
                else {
                    return;
                };
                self.named.insert(id);
                named.push(Element::Node {
                    id,
                    at: position_on(self.grid, latitude, longitude),
                    tags: tags
                        .iter()
                        .map(|(key, value)| {
                            (
                                text(decoded.string(*key as usize)),
                                text(decoded.string(*value as usize)),
                            )
                        })
                        .collect(),
                });
            };
            match name_key {
                None => dense.visit(&mut |id, latitude, longitude| keep(id, latitude, longitude, None))?,
                Some(_) => dense.visit_tagged(&mut |id, latitude, longitude, tags| {
                    keep(id, latitude, longitude, Some(tags));
                })?,
            }
            self.positions
                .write(&stored)
                .map_err(|error| self.failure.keep(error))?;
            for node in &named {
                if self
                    .elements
                    .add(node)
                    .map_err(|error| self.failure.keep(error))?
                {
                    self.kept.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        Ok(None)
    }
}

/// The pass over every way: each way with a node inside the countries set down whole, every node
/// the planet places, and each member of a multipolygon set apart by its id.
struct WayPass<'a> {
    inside: &'a IdentifierSet,
    member_ways: &'a IdentifierSet,
    positions: &'a NodePositions,
    grid: Grid,
    referenced: &'a IdentifierSet,
    kept_ways: &'a IdentifierSet,
    elements: &'a SetDown<'a>,
    members: &'a Spill,
    kept: &'a AtomicUsize,
    failure: &'a WorkFailure,
}

impl WayPass<'_> {
    fn visit(&self, decoded: &Block<'_>) -> Result<Option<planetrs_pbf::IdSpan>, PbfError> {
        let mut wanted: Vec<(i64, Tags, Vec<i64>, bool, bool)> = Vec::new();
        for group in &decoded.groups {
            visit_ways(group, &mut |way| {
                let refs = way.refs().collect::<Result<Vec<i64>, _>>()?;
                let touches = refs.iter().any(|node| self.inside.contains(*node));
                let member = self.member_ways.contains(way.id);
                if touches || member {
                    wanted.push((way.id, named_tags(way.tags(decoded)), refs, touches, member));
                }
                Ok(())
            })?;
        }
        let every_ref: Vec<i64> = wanted
            .iter()
            .flat_map(|(_, _, refs, _, _)| refs.iter().copied())
            .collect();
        let found = self
            .positions
            .read(&every_ref)
            .map_err(|error| self.failure.keep(error))?;
        let mut cursor = 0;
        for (id, tags, refs, touches, member) in wanted {
            let positions = found[cursor..cursor + refs.len()]
                .iter()
                .map(|at| at.map(|(latitude, longitude)| position_on(self.grid, latitude, longitude)))
                .collect();
            cursor += refs.len();
            let line = Line {
                id,
                tags,
                refs,
                positions,
            };
            if member {
                let by_id = Placement {
                    key: id as u64,
                    order: (1, id),
                    weight: 0,
                    rows: 0,
                };
                self.members
                    .add(&by_id, &Element::Way(line.clone()))
                    .map_err(|error| self.failure.keep(error))?;
            }
            if touches {
                self.kept_ways.insert(id);
                for node in &line.refs {
                    self.referenced.insert(*node);
                }
                if self
                    .elements
                    .add(&Element::Way(line))
                    .map_err(|error| self.failure.keep(error))?
                {
                    self.kept.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        Ok(None)
    }
}

/// How many tiles are written at once, each on its own thread: encoding a tile is the slow part.
const TILES_WRITTEN_AT_ONCE: usize = 8;

/// How many elements wait for a tile's writer before the merge waits for it.
const ELEMENTS_IN_FLIGHT: usize = 4_096;

/// Writes each tile of `tiles` beside the dataset from the elements merged back from `spill`, the
/// merge handing each element to its tile's writer while several tiles are encoded at once.
fn write_tiles(
    tiles: Vec<(Tile, std::ops::Range<u64>)>,
    spill: Spill,
    directory: &Path,
    work: &Work,
    grid: Grid,
) -> Result<Vec<Written>, ClipError> {
    std::thread::scope(|scope| {
        let mut merged = spill.merged().map_err(|e| work.failed(e))?.peekable();
        let mut writing: VecDeque<TileUnderWay<'_>> = VecDeque::new();
        let mut written = Vec::with_capacity(tiles.len() + 1);
        for (tile, keys) in tiles {
            if writing.len() == TILES_WRITTEN_AT_ONCE
                && let Some(oldest) = writing.pop_front()
            {
                written.push(oldest.finished()?);
            }
            let name = tile.file_name();
            let beside = written_beside(directory, &name);
            let file = File::create(&beside).map_err(|failure| ClipError::writing(&beside, failure))?;
            let (hand, take) = std::sync::mpsc::sync_channel::<spill::Spilled>(ELEMENTS_IN_FLIGHT);
            let writer = scope.spawn(move || {
                let mut writer = TileWriter::new(BufWriter::new(file), tile, grid);
                for spilled in take {
                    let element = spilled
                        .element()
                        .map_err(|e| ClipError::writing(work.path(), e))?;
                    writer.add(element).map_err(ClipError::Encoding)?;
                }
                writer.finish().map_err(ClipError::Encoding)
            });
            while let Some(next) =
                merged.next_if(|next| next.as_ref().map_or(true, |spilled| keys.contains(&spilled.key)))
            {
                if hand.send(next.map_err(|e| work.failed(e))?).is_err() {
                    break;
                }
            }
            drop(hand);
            writing.push_back(TileUnderWay {
                tile,
                name,
                beside,
                writer,
            });
        }
        for under_way in writing {
            written.push(under_way.finished()?);
        }
        Ok(written)
    })
}

/// A tile being encoded on its own thread.
struct TileUnderWay<'scope> {
    tile: Tile,
    name: String,
    beside: PathBuf,
    writer: std::thread::ScopedJoinHandle<'scope, Result<TileWritten<BufWriter<File>>, ClipError>>,
}

impl TileUnderWay<'_> {
    /// Waits for the tile, puts it on disk, and gives back its entry in the manifest.
    fn finished(self) -> Result<Written, ClipError> {
        let writing = |failure| ClipError::writing(&self.beside, failure);
        let done = self
            .writer
            .join()
            .map_err(|_| writing(io::Error::other("the tile's writer stopped")))??;
        done.out
            .into_inner()
            .map_err(|failure| writing(failure.into_error()))?
            .sync_all()
            .map_err(writing)?;
        Ok(Written {
            part: Part::MapTile {
                level: self.tile.level,
                x: self.tile.x,
                y: self.tile.y,
            },
            name: self.name,
            rows: done.rows,
            bbox: Some(done.bbox),
        })
    }
}

/// Sets down every multipolygon with a member among the kept ways, each with its member ways whole
/// in the order its members first name them, joined from the members set apart by id: how many.
fn assemble_multipolygons(
    multipolygons: &[MultipolygonKept],
    kept_ways: &IdentifierSet,
    members: Spill,
    work: &Work,
    grid: Grid,
    elements: &SetDown<'_>,
) -> Result<usize, ClipError> {
    let kept: Vec<&MultipolygonKept> = multipolygons
        .iter()
        .filter(|multipolygon| {
            multipolygon
                .members
                .iter()
                .any(|(way, _)| kept_ways.contains(*way))
        })
        .collect();
    let mut memberships: Vec<(i64, usize, usize)> = kept
        .iter()
        .enumerate()
        .flat_map(|(at, multipolygon)| {
            first_named(multipolygon.members.iter().map(|(way, _)| *way))
                .into_iter()
                .enumerate()
                .map(move |(order, way)| (way, at, order))
        })
        .collect();
    memberships.sort_unstable();
    let assembled = Spill::new(&work.join("assembled"), grid, RUN_BYTES).map_err(|e| work.failed(e))?;
    let mut next = 0;
    for member in members.merged().map_err(|e| work.failed(e))? {
        let member = member.map_err(|e| work.failed(e))?;
        let (way, line) = (member.key as i64, member.element().map_err(|e| work.failed(e))?);
        while next < memberships.len() && memberships[next].0 < way {
            next += 1;
        }
        let mut at = next;
        while at < memberships.len() && memberships[at].0 == way {
            let (_, multipolygon, order) = memberships[at];
            let place = Placement {
                key: multipolygon as u64,
                order: (0, order as i64),
                weight: 0,
                rows: 0,
            };
            assembled.add(&place, &line).map_err(|e| work.failed(e))?;
            at += 1;
        }
    }
    let mut count = 0;
    let mut gathered: Option<(usize, Vec<Line>)> = None;
    let mut set_down = |(at, ways): (usize, Vec<Line>)| -> Result<(), ClipError> {
        let multipolygon = kept[at];
        let element = Element::Multipolygon {
            id: multipolygon.id,
            tags: multipolygon.tags.clone(),
            members: multipolygon.members.clone(),
            ways,
        };
        if elements.add(&element).map_err(|e| work.failed(e))? {
            count += 1;
        }
        Ok(())
    };
    for line in assembled.merged().map_err(|e| work.failed(e))? {
        let line = line.map_err(|e| work.failed(e))?;
        let at = line.key as usize;
        let Element::Way(line) = line.element().map_err(|e| work.failed(e))? else {
            continue;
        };
        match gathered.as_mut() {
            Some((gathering, ways)) if *gathering == at => ways.push(line),
            _ => {
                if let Some(done) = gathered.replace((at, vec![line])) {
                    set_down(done)?;
                }
            }
        }
    }
    if let Some(done) = gathered {
        set_down(done)?;
    }
    Ok(count)
}

/// A multipolygon with its way members.
struct MultipolygonKept {
    id: i64,
    tags: Tags,
    members: Vec<(i64, String)>,
}

fn visit_multipolygons(
    found: &mut Vec<MultipolygonKept>,
    decoded: &Block<'_>,
) -> Result<Option<planetrs_pbf::IdSpan>, PbfError> {
    for group in &decoded.groups {
        visit_relations(group, &mut |relation| {
            let tags = named_tags(relation.tags(decoded));
            if tag(&tags, "type") != Some(MULTIPOLYGON) {
                return Ok(());
            }
            found.push(MultipolygonKept {
                id: relation.id,
                tags,
                members: relation
                    .ways(decoded)
                    .into_iter()
                    .map(|(way, role)| (way, text(role)))
                    .collect(),
            });
            Ok(())
        })?;
    }
    Ok(None)
}

/// The grid the first node block stores positions on, which every other must share.
fn first_grid(planet: &Planet, section: std::ops::Range<usize>) -> Result<Grid, ClipError> {
    let Some(first) = (!section.is_empty()).then_some(section.start) else {
        return Err(ClipError::Grid {
            detail: "the planet holds no node".into(),
        });
    };
    let mut payload = Vec::new();
    let mut inflated = Vec::new();
    planet.inflate_blob(first, &mut payload, &mut inflated)?;
    let decoded = block(&inflated)?;
    Ok(Grid {
        granularity: decoded.granularity,
        latitude_offset: decoded.lat_offset,
        longitude_offset: decoded.lon_offset,
    })
}

/// The one grid every node block stores positions on, refused when there are several or when it is
/// finer than a map index holds.
fn grid_of(grids: &[Grid], out_of_range: bool) -> Result<Grid, ClipError> {
    match grids {
        [grid] if grid.granularity >= FINEST_GRANULARITY && !out_of_range => Ok(*grid),
        [grid] => Err(ClipError::Grid {
            detail: format!(
                "a unit of {} nanodegrees is finer than the {FINEST_GRANULARITY} a map index holds",
                grid.granularity
            ),
        }),
        [] => Err(ClipError::Grid {
            detail: "the planet holds no node".into(),
        }),
        _ => Err(ClipError::Grid {
            detail: format!(
                "its node blocks use {} grids, and a map index holds one",
                grids.len()
            ),
        }),
    }
}

fn position_on(grid: Grid, latitude: i32, longitude: i32) -> Position {
    Position {
        latitude: i64::from(latitude) * grid.granularity + grid.latitude_offset,
        longitude: i64::from(longitude) * grid.granularity + grid.longitude_offset,
    }
}

/// The largest id of a section, read from its last blob.
fn largest_id(planet: &Planet, section: std::ops::Range<usize>) -> Result<i64, PbfError> {
    let Some(last) = section.end.checked_sub(1).filter(|last| *last >= section.start) else {
        return Ok(0);
    };
    let mut payload = Vec::new();
    let mut inflated = Vec::new();
    planet.inflate_blob(last, &mut payload, &mut inflated)?;
    let decoded = block(&inflated)?;
    Ok(id_span(&decoded)?.map(|span| span.last).unwrap_or(0))
}

/// Each id once, in the order it is first named.
fn first_named(ids: impl IntoIterator<Item = i64>) -> Vec<i64> {
    let mut seen = Vec::new();
    for id in ids {
        if !seen.contains(&id) {
            seen.push(id);
        }
    }
    seen
}

fn concatenated<T>(mut left: Vec<T>, right: Vec<T>) -> Vec<T> {
    left.extend(right);
    left
}

fn named_tags(tags: Vec<(&[u8], &[u8])>) -> Tags {
    tags.into_iter()
        .map(|(key, value)| (text(key), text(value)))
        .collect()
}

/// Text from the planet's bytes, as a harvest reads it: invalid UTF-8 replaced rather than refused.
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
