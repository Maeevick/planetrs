# planetrs

[![CI](https://github.com/maeevick/planetrs/actions/workflows/ci.yml/badge.svg)](https://github.com/maeevick/planetrs/actions/workflows/ci.yml)
[![planetrs-geocoder](https://img.shields.io/crates/v/planetrs-geocoder.svg?label=planetrs-geocoder)](https://crates.io/crates/planetrs-geocoder)
[![planetrs-mapclipper](https://img.shields.io/crates/v/planetrs-mapclipper.svg?label=planetrs-mapclipper)](https://crates.io/crates/planetrs-mapclipper)
[![Code: MIT](https://img.shields.io/badge/code-MIT-blue.svg)](LICENSE)
[![Data: ODbL 1.0](https://img.shields.io/badge/data-ODbL%201.0-green.svg)](https://www.openstreetmap.org/copyright)

Three goblins and one tired human read the whole of OpenStreetMap, ninety-odd gigabytes of it, so
that nobody has to ask a tile server for permission. Out of it they make two things: a coordinate for
a postal address that says how far it can be trusted, and a small illustrated map of anywhere. No
API, no key, no rate limit, and no network once the planet is on disk.[^1]

![Monaco, as planetrs-mapclipper draws it with the atlas theme](docs/monaco-atlas.png)

## Meet the staff

**Grimbleshanks Mortimer Quill**, Archivist of Neglected Knowledge and Assistant Deputy Custodian of
the Great Library of Cogsburg, reads the planet one blob at a time and files what he finds: every
commune on Earth, and every street, door, hamlet and shop of France and Monaco, in tab-separated files
sorted so that two readings of one planet agree to the byte. He keeps `planetrs-pbf`, `planetrs-geo`
and the index, and reminds visitors that people change but books remain constant.[^2]

**Veil** hunts. Hand Veil an address typed in a hurry, `ZA des Rivières - 46 Rue de Lisbonne`, or
`13 Cours du Général Vauban` for a street that was always the Maréchal's, and Veil comes back with
the street it meant and the door if the planet knows it. What Veil will not do is guess. An answer
that only reached the middle of a town says so, because a town centre written as a coordinate looks
exactly like an address and is not one. Veil runs `planetrs-geocoder`, and would rather you were
quiet about it.[^3]

**Trix**, the Chaotic Apprentice, was expelled from several academies for "inappropriate
experimentations with infrastructure" and now cuts maps out of the planet instead, which is the only
infrastructure anyone lets her near. She runs `planetrs-mapclipper`. Its three themes are fewer than
the colours her hair has been this week.[^4]

**maeevick** is the tall intern: carries the planet file between machines, writes the tests the
goblins insist on, and apologises to the laptop fan.

## What is in the box

| | |
| --- | --- |
| [`planetrs-geocoder`](geocoder/README.md) | a coordinate for a place or a postal address, and how precise it is: the door, the nearest door, a shop of the label, the street, or the commune's centre |
| [`planetrs-mapclipper`](mapclipper/README.md) | a small illustrated map of anywhere, as GeoJSON, SVG or PNG, in themes that are files rather than code |

Both read the OpenStreetMap planet file directly, and both are libraries as well as commands.

## Install

Release archives for macOS (Apple silicon and Intel) and Linux (x86_64 and aarch64, static):

```sh
target=aarch64-apple-darwin   # or x86_64-apple-darwin, x86_64-unknown-linux-musl, aarch64-unknown-linux-musl
curl -fsSL "https://github.com/maeevick/planetrs/releases/latest/download/planetrs-geocoder-$target.tar.gz" | tar -xz
install -m 755 "planetrs-geocoder-$target/planetrs-geocoder" ~/.local/bin/
```

`planetrs-mapclipper` comes the same way, or either command from source with Cargo:

```sh
cargo install --git https://github.com/maeevick/planetrs planetrs-geocoder-cli
cargo install --git https://github.com/maeevick/planetrs planetrs-mapclipper-cli
```

A binary downloaded through a browser is quarantined by macOS until `xattr -d com.apple.quarantine
planetrs-geocoder`. The binaries are not notarised yet; `curl` sidesteps the question.

## Try it on something small

Monaco is a two-square-kilometre planet that downloads in a second:

```sh
planetrs-geocoder download --from https://download.geofabrik.de/europe/monaco-latest.osm.pbf
planetrs-geocoder index --planet ~/.local/share/planetrs/planets/monaco-latest.osm.pbf --countries MC
planetrs-geocoder "Musée océanographique;Avenue Saint-Martin;98000;Monaco;Monaco"
```

```text
label;address;postal code;city;country;lat;lon;precision
Musée océanographique;Avenue Saint-Martin;98000;Monaco;Monaco;43.731809;7.427267;Street
```

The map above came out of the same file, indexed once and then cut from the dataset:

```sh
planetrs-mapclipper index --planet ~/.local/share/planetrs/planets/monaco-latest.osm.pbf --countries MC
planetrs-mapclipper clip \
  --lat 43.7375 --lon 7.4210 --width-km 3 --height-km 2 --width-px 900 --height-px 600 \
  --theme atlas --labels settlements --marker name --name "en=Monaco" --png monaco.png
```

The real thing is the same three commands without the arguments. `download` fetches the latest
planet, about 90 GB, resumes when the connection breaks, and checks its md5 before the file takes its
name. `index` then needs about 20 GB of memory and four minutes on a recent laptop for France and
Monaco, after which a whole file of sites geocodes in seconds:

```sh
planetrs-geocoder download
planetrs-geocoder index
planetrs-geocoder -f sites.csv -o answers.csv
```

## The workspace

```text
crates/planetrs-pbf                  reading PBF: protobuf, framing, blocks, file header, an id index
crates/planetrs-geo                  coordinates, bounding boxes, WKB, the window a block is tested against
crates/planetrs-dataset              the dataset both tools read: a manifest, a gazetteer, GeoParquet indexes
crates/planetrs-cli                  what both command lines share: data directory, outputs, progress, exit codes
geocoder/planetrs-geocoder           the geocoding library
geocoder/planetrs-geocoder-cli       the planetrs-geocoder command line
mapclipper/planetrs-mapclipper       the map library
mapclipper/planetrs-mapclipper-cli   the planetrs-mapclipper command line
```

The crates under `crates/` are shared and interpret no tag. What a `highway` means belongs to
whoever reads it.

## Why not a service

The planet file has no spatial index, so every question is a full scan. That is fine as a batch job
and impossible as a request, which makes a command line the honest shape for this rather than a
hosted API.

It is also the shape the work needs. A coordinate that will be relied on after the fact has to be
defensible, and that means every step from the published planet file to the coordinate must be one
somebody else can read and re-run. Determinism against a pinned snapshot is worth more here than a
hundred metres of median accuracy, and an answer that says it is only a town centre is worth more
than one that hides it.

## Building and testing

```sh
make check          # format, lint at -D warnings, then the tests
make test-unit      # pure logic, no filesystem
make test-integrated
make test-system    # against a real planet file, PLANET=/path/to/planet.osm.pbf
make msrv doc deny package
```

Rust 1.88 or later. See [CONTRIBUTING.md](CONTRIBUTING.md) for the rest of the house rules, which
the goblins enforce with more enthusiasm than tact.

## Data and licence

The code is MIT, see [LICENSE](LICENSE).

The data is not ours. OpenStreetMap is © OpenStreetMap contributors and available under the Open Data
Commons Open Database License ([ODbL](https://opendatacommons.org/licenses/odbl/1-0/)) by the
OpenStreetMap Foundation (OSMF): <https://www.openstreetmap.org/copyright>. planetrs follows it for
everything it derives from the planet:

- **An index** built by `planetrs-geocoder index` is a Derivative Database. Share one and you share it
  under the ODbL, with the notice every one of its files carries, naming the licence and linking to
  it as data has to.
- **Geocoding results** are, under the OSMF's geocoding guideline, insubstantial extracts one at a
  time, so they bring no share-alike; wherever you show them, credit OpenStreetMap. The command line
  ends every run with that credit.
- **Maps** from `planetrs-mapclipper` are Produced Works. A picture holds no link, so each one shows
  `© OpenStreetMap contributors` and the copyright page's full address on the picture itself, and an
  SVG links it too. A GeoJSON or a kept harvest is a Derivative Database, as an index is, and names
  the licence and links to it.
- **Third-party crates** keep their own licences, and every release archive ships them in
  `THIRD-PARTY-LICENSES.html`.

[^1]: Once it is on disk. Downloading it is a few hours of somebody else's bandwidth, so the planet
    server would rather you used a mirror: `download --from` takes any of them.

[^2]: He also shelves demonology beside gardening, both being about things that grow in the dark.
    The index files communes by OpenStreetMap relation id, a compromise he has catalogued under
    Regrettable, Necessary.

[^3]: Shhhhh. Veil is hunting.

[^4]: Grimble first took her for the maintenance staff. In his defence, something nearby was usually
    on fire.
