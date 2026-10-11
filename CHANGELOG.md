# Changelog

Every notable change, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the version numbers follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html): until 1.0,
a minor version may break the API or the index format.

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

[0.1.0]: https://github.com/maeevick/planetrs/releases/tag/v0.1.0
