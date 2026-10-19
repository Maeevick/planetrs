# planetrs-pbf

Reading OpenStreetMap PBF files: the framing, the blocks, and an id index over a sorted planet.

Grimbleshanks Mortimer Quill, Archivist of Neglected Knowledge and keeper of the
[planetrs](https://github.com/maeevick/planetrs) goblins' library, reads the planet with this crate,
one blob at a time, and does not trust a single page he has not checked against its own header.

It reads a planet file and nothing else: no API, no tile service, no network. Two properties of the
format shape everything in it. There is **no spatial index**, so any question about a place is a full
scan of the file. And the planet declares `Sort.Type_then_ID`, so every node precedes every way, ids
ascend within a section, and both the bisection into node, way and relation sections and the id
pruning of `IdIndex` are sound. That sort order is checked against the file's own header rather than
assumed: a file that does not declare it is refused.

```rust
use planetrs_pbf::Planet;

let planet = Planet::open("planet-260907.osm.pbf".as_ref())?;
let sections = planet.sections()?;
println!("{} blobs, ways from blob {}", planet.blobs.len(), sections.ways.start);
# Ok::<(), planetrs_pbf::PbfError>(())
```

The `testing` feature writes small planet files for tests, a header and sorted blocks of nodes, ways
and relations, so that whatever reads a planet can be tested on a handful of nodes rather than on
ninety gigabytes.

The system tests read a real planet and run on request:

```sh
PLANET=/path/to/planet.osm.pbf cargo test -p planetrs-pbf --release -- --ignored
```

## Data and licence

The code is MIT ([LICENSE](https://github.com/maeevick/planetrs/blob/main/LICENSE)). The data it
reads is © OpenStreetMap contributors and available under the Open Data Commons Open Database License
(ODbL) by the OpenStreetMap Foundation (OSMF): <https://www.openstreetmap.org/copyright>.
