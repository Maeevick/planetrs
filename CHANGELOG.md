# Changelog

Every notable change, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the version numbers follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html): until 1.0,
a minor version may break the API or the dataset format.

## [0.2.0] - 2026-10-20

Both tools now read a dataset built once from the planet, from a directory or over HTTP, rather than
the planet itself or a tab-separated index. 0.1.0 was never tagged; its notes below describe `main`
as it was first published.

### planetrs-geocoder

- The tab-separated index gives way to the dataset. Opening reads the gazetteer, and a lookup reads
  its country's word counts once and its commune with the communes stored beside it, from a
  directory or over HTTP (`--dataset DIR|URL`, `PLANETRS_DATASET`): a first lookup in France takes
  well under a second and about 500 MB.
- Word rarity is counted over the streets of the site's country.
- `index` builds the gazetteer and each country's geocoder index beside the dataset, keeps the map
  indexes built from the same planet, and stops at the first blob that does not decode rather than
  counting it.
- `trace` looks for the same street name within 20 km on both sides of a border.
- `Geocoder::locate` and `Geocoder::trace` take `&mut self` and return a `Result`, since a lookup can
  now fail on the network.

### planetrs-mapclipper

- A library, `planetrs-mapclipper`, and the `planetrs-mapclipper` command, now in
  `planetrs-mapclipper-cli`.
- `index` files the ways, named and lone nodes and multipolygons of the countries asked for, or of
  the whole planet with `--world`, into a map index cut into tiles of about a gigabyte. `harvest` and
  `clip` then cut a box from the tiles around it, the planet's own harvest to the byte for any box
  the map index covers, and refuse a box that reaches beyond it; `--planet PATH` reads the planet
  instead.
- The build holds little in memory: node positions in a file beside the dataset, elements set down
  on disk in sorted runs, tiles encoded sixteen row groups at a time. France and Monaco take about
  eleven minutes and 17 GB of memory; the whole planet an hour and a quarter and 33 GB.
- `download` fetches the planet, as the geocoder's does, so the mapclipper needs no other tool to
  index locally.
- The command line follows the geocoder's conventions: `--data-dir`, `--dataset`, `-q` for every
  command, the next step after an error, 0 on a closed pipe, and the attribution after every run that
  reads OpenStreetMap data. Maps and harvests are never written over.
- The `.mapclip-index` sidecar and `--index` are gone; the id index lives for one harvest.

### planetrs-dataset and planetrs-cli, new

- The dataset: a manifest with each file's checksum and each tile's box, a planet-wide gazetteer, a
  geocoder index per country, and a map index as a summary and its tiles, all GeoParquet 1.1, the
  same bytes from the same planet, read from a directory or over HTTP range requests through a cache.
  A file holds its content alone, the planet named by the manifest, so a tile that did not change
  keeps its bytes from one planet to the next.
- What both command lines share: the data directory, the planet's download, outputs never written
  over, the progress line, exit codes and the OpenStreetMap attribution.

### planetrs-pbf and planetrs-geo

- `planetrs-pbf` writes small planet files for tests, behind its `testing` feature.
- `planetrs-geo` gains country outlines, the cells a set of countries covers whole, and WKB for
  points and lines.

## [0.1.0] - 2026-10-10

The first public release.

### planetrs-geocoder

- A coordinate for one site given as `"label;address;postal code;city;country"`, or for a whole
  semicolon-separated file, against an index built from the OpenStreetMap planet.
- Every answer carries its precision: `HouseNumber`, `NearestHouseNumber`, `Shop`, `Street`,
  `Commune` or `NotFound`, and an answer at a commune's centre is flagged for a person to look at.
- Reading rules for French addresses: door ranges and suffixes, designators, spaced dashes, `et` and
  `and`, road numbers, typos and sound-alike spellings, and rewriting an address that found nothing.
- `download` fetches the latest planet, resumes a broken download and checks its md5; `index` builds
  the index beside the old one and swaps it in once complete; `trace` prints every decision; `status`
  shows the data directory.
- Answer files are never written over.

### planetrs-mapclipper

- A small illustrated map of anywhere, as GeoJSON, SVG or PNG, from a harvest that can be rendered
  again at another framing or in another theme.
- Themes are TOML files; names are placed so that the strongest claim wins a crowded corner.
- Every SVG and PNG shows `© OpenStreetMap contributors` and the copyright page's full address where
  no label can cover them; GeoJSON files and harvests name the ODbL and link to it.

### planetrs-pbf and planetrs-geo

- A PBF reader that checks the planet's sort order, bisects its sections and indexes blob id spans.
- Boxes asked for in kilometres, and the integer windows a block is tested against.

[0.2.0]: https://github.com/maeevick/planetrs/releases/tag/v0.2.0
[0.1.0]: https://github.com/maeevick/planetrs/tree/dd55304
