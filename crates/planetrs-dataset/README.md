# planetrs-dataset

The dataset both [planetrs](https://github.com/maeevick/planetrs) tools read instead of the planet: a
manifest, a planet-wide gazetteer, a geocoder index per country and a map index cut into tiles, read
from its directory a few row groups at a time.

A dataset is built whole from one planet file, so every file in it answers for the same minute of
OpenStreetMap history, and two builds from one planet agree to the byte.

| File | What it holds |
| --- | --- |
| `manifest.json` | the planet it was built from, the licence, and each file with its size, checksum and rows |
| `gazetteer.parquet` | every administrative area of the planet, with its names, postcodes, area and centre |
| `geocoder-FR.parquet` | for one country, its word counts, then the doors, middles and shops of each commune |
| `map-summary.json` | the map index as a whole: the countries it was built for or the whole planet, the grid of its positions and the ground it covers |
| `map-7-65-96.parquet` | one tile of the map index, a cell of a quadtree over the planet: every way, named or lone node and multipolygon a map needs there, along a Hilbert curve |

Every Parquet file is GeoParquet 1.1, which DuckDB, GDAL and the rest read as they are. A reader opens
only what it needs: a commune's row group in a geocoder index, and in the map index the tiles whose
box in the manifest touches a place, then the row groups near it.

```rust
use planetrs_dataset::{Dataset, DirectoryFiles};

let dataset = Dataset::open(DirectoryFiles::at("dataset"))?;
let rock_of_monaco = [7.40, 43.72, 7.44, 43.75];
for tile in dataset.map_tiles_near(rock_of_monaco)? {
    let near = dataset.elements_near(&tile, rock_of_monaco)?;
    println!("{}: {} elements near the Rock of Monaco", tile.tile().file_name(), near.len());
}
# Ok::<(), planetrs_dataset::DatasetError>(())
```

The `testing` feature brings files held in memory, and the contract that they and a directory both
honour.

## Licence

The code is MIT ([LICENSE](https://github.com/maeevick/planetrs/blob/main/LICENSE)). The data it reads
and writes is © OpenStreetMap contributors, under the Open Data Commons Open Database License 1.0.
