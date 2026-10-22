# planetrs-dataset

The dataset both [planetrs](https://github.com/maeevick/planetrs) tools read instead of the planet: a
manifest, a planet-wide gazetteer, and one geocoder index and one map index per country, all
GeoParquet, read from a directory or over HTTP range requests.

A dataset is built whole from one planet file, so every file in it answers for the same minute of
OpenStreetMap history, and two builds from one planet agree to the byte.

| File | What it holds |
| --- | --- |
| `manifest.json` | the planet it was built from, the licence, and each file with its size, checksum and rows |
| `gazetteer.parquet` | every administrative area of the planet, with its names, postcodes, area and centre |
| `geocoder-FR.parquet` | for one country, its word counts, then the doors, middles and shops of each commune |
| `map-summary.json` | the map index as a whole: the countries it was built for or the whole planet, the grid of its positions and the ground it covers |
| `map-7-65-96.parquet` | one tile of the map index, a cell of a quadtree over the planet: every way, named or lone node and multipolygon a map needs there, along a Hilbert curve |

Every file is GeoParquet 1.1, which DuckDB, GDAL and the rest read as they are. A reader picks the
row groups it needs by their statistics: a commune's in a geocoder index, the ones near a place in a
map index.

```rust
use planetrs_dataset::{Dataset, DirectoryFiles};

let dataset = Dataset::open(DirectoryFiles::at("dataset"))?;
for index in dataset.map_indexes()? {
    let near = dataset.elements_near(&index, [7.40, 43.72, 7.44, 43.75])?;
    println!("{}: {} elements near the Rock of Monaco", index.country(), near.len());
}
# Ok::<(), planetrs_dataset::DatasetError>(())
```

The `remote` feature reads a dataset over HTTP (`HttpFiles`), holding each file to the ETag it was
first read at, and `Dataset::cached_in` keeps what was read on disk. The `testing` feature brings
files held in memory, a small server that honours ranges, and the contract every way of reading files
passes.

## Licence

The code is MIT ([LICENSE](https://github.com/maeevick/planetrs/blob/main/LICENSE)). The data it reads
and writes is © OpenStreetMap contributors, under the Open Data Commons Open Database License 1.0.
