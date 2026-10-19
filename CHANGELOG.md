# Changelog

Every notable change, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the version numbers follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html): until 1.0,
a minor version may break the API or the dataset format.

## [0.1.0] - 2026-10-10

The first public release.

### planetrs-geocoder

- A coordinate for one site given as `"label;address;postal code;city;country"`, or for a whole
  semicolon-separated file, against a dataset built from the OpenStreetMap planet, read commune by
  commune from a directory or over HTTP (`--dataset DIR|URL`, `PLANETRS_DATASET`).
- Every answer carries its precision: `HouseNumber`, `NearestHouseNumber`, `Shop`, `Street`,
  `Commune` or `NotFound`, and an answer at a commune's centre is flagged for a person to look at.
- Reading rules for French addresses: door ranges and suffixes, designators, spaced dashes, `et` and
  `and`, road numbers, typos and sound-alike spellings, and rewriting an address that found nothing.
- `download` fetches the latest planet, resumes a broken download and checks its md5; `index` builds
  the gazetteer and each country's geocoder index beside the dataset, keeps the map indexes built from
  the same planet, and swaps it in once complete; `trace` prints every decision; `status` shows the
  data directory and the dataset.
- Answer files are never written over.

### planetrs-mapclipper

- A small illustrated map of anywhere, as GeoJSON, SVG or PNG, from a harvest that can be rendered
  again at another framing or in another theme.
- `index` files each country's ways, named and lone nodes and multipolygons into its map index;
  `harvest` and `clip` then cut a box from the dataset, the planet's own harvest to the byte for any
  box inside the countries indexed, and refuse a box reaching beyond them. `--planet PATH` reads the
  planet instead.
- A library, `planetrs-mapclipper`, and the `planetrs-mapclipper` command in `planetrs-mapclipper-cli`.
  Maps and harvests are never written over.
- Themes are TOML files; names are placed so that the strongest claim wins a crowded corner.
- Every SVG and PNG shows `© OpenStreetMap contributors` and the copyright page's full address where
  no label can cover them; GeoJSON files and harvests name the ODbL and link to it.

### planetrs-dataset and planetrs-cli

- The dataset both tools read: a manifest with each file's checksum, a planet-wide gazetteer, and a
  geocoder index and a map index per country, all GeoParquet 1.1, the same bytes from the same planet,
  read from a directory or over HTTP range requests through a cache.
- What both command lines share: the data directory, outputs never written over, the progress line,
  exit codes and the OpenStreetMap attribution.

### planetrs-pbf and planetrs-geo

- A PBF reader that checks the planet's sort order, bisects its sections and indexes blob id spans.
- Boxes asked for in kilometres, the integer windows a block is tested against, country outlines and
  the ground a dataset covers, and WKB for points and lines.

[0.1.0]: https://github.com/maeevick/planetrs/releases/tag/v0.1.0
