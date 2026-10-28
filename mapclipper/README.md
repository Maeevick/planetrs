# planetrs-mapclipper

Cutting a small illustrated map out of the OpenStreetMap planet file, offline. No tile service, no
API, no network at run time.

It began as a way to avoid asking a tile server for one small picture and grew, as these things do,
into a library, a command line and a map index of the whole planet. It is one half of
[planetrs](https://github.com/maeevick/planetrs), and so far nothing has caught fire.

![Monaco, drawn with the atlas theme](https://raw.githubusercontent.com/maeevick/planetrs/main/docs/monaco-atlas.png)

```sh
planetrs-mapclipper clip --planet planet-260907.osm.pbf \
  --lat 5.269 --lon -4.012 --width-km 29 --height-km 16.53 \
  --width-px 1000 --height-px 570 --theme linework \
  --marker name --name "en=Port of Abidjan" \
  --png abidjan.png
```

## Two verbs, and why

The work splits in two, and the split is the design rather than an optimisation.

**Harvesting** reads the planet. A PBF file has no spatial index, so every question about a place is
a full scan of ninety-four gigabytes. It is minutes.

**Rendering** draws what was harvested. It is milliseconds, and it can be asked again at another
framing, another canvas, another theme.

```sh
planetrs-mapclipper harvest --planet planet.osm.pbf --lat 5.269 --lon -4.012 \
  --width-km 40 --height-km 40 --out abidjan.mapclip     # once

planetrs-mapclipper render abidjan.mapclip --lat 5.269 --lon -4.012 \
  --width-km 29 --height-km 16.53 --width-px 1000 --height-px 570 \
  --theme linework --svg abidjan.svg                      # as often as you like
```

Tying them together makes every question about colour or framing cost a full pass, which is the
difference between trying six framings and settling for the first.

`clip` does both in one go, for the first time you look at a place.

## A dataset instead of the planet

A harvest of the planet reads ninety-four gigabytes for a box a few kilometres across. `index` reads
them once instead, and files every way, named or lone node and multipolygon of the countries asked
for, or of the whole planet with `--world`, into a map index in the dataset the geocoder shares, cut
into tiles of about a gigabyte:

```sh
planetrs-mapclipper download                               # the latest planet, resumable
planetrs-mapclipper index --countries FR,MC               # once, from the newest planet
planetrs-mapclipper clip --lat 43.7375 --lon 7.4210 \
  --width-km 3 --height-km 2 --png monaco.png              # in a moment, from the dataset
```

`harvest` and `clip` then cut their box from the dataset, reading only the tiles and the row groups
around it. For any box the map index covers, the harvest is the planet's own to the byte; `--planet
PATH` reads the planet instead, for anywhere else. That claim was checked on five places in France
and Monaco, and on six around the world from one build of the whole planet; other countries go
through the same code, and nobody has looked at them yet. `harvest` from the dataset set against
`harvest --planet`, for a box of your own, makes a fine report: see
[CONTRIBUTING.md](../CONTRIBUTING.md).

The build holds little in memory: every node's position goes into a file beside the dataset, every
element is set down on disk as it is built, and the tiles are written from them merged back in order.
France and Monaco take about eleven minutes, 17 GB of memory and 130 GB of scratch disk; the whole
planet an hour and a quarter, 33 GB of memory and 220 GB of scratch disk, for 614 tiles and 225 GB.

A map index records the ground it answers for: everywhere, for the whole planet, or the cells of a
hundredth of a degree, about a kilometre, that lie wholly inside the countries indexed, a border
between two of them counting as inside. A box that comes within a cell of a border with a country
not indexed is refused rather than cut with holes, and the error says what to do instead: read the
planet with `--planet PATH`, or index the countries the box reaches as well.

## A harvest is a thing you can give away

A `.mapclip` is a few tens of megabytes and holds everything the planet has for one area. It is
versioned, it records the exact planet file it was cut from and names the licence its data is under,
and **it is byte-deterministic**: the same planet and the same area produce the same bytes, so a
checksum over one means something.

That matters because it makes the expensive half transferable. Somebody with a harvest can render
from it with no planet file at all.

Under ODbL a harvest is a **derivative database**, so publishing one carries share-alike.

## What it draws

| Output | What it is | Under ODbL |
| ------ | ---------- | ---------- |
| `--geojson` | lon/lat, never projected, as RFC 7946 requires | a derivative database: share-alike |
| `--svg` | Web Mercator, one path per feature, grouped by category | a produced work: attribution |
| `--png` | the SVG rasterised through resvg, in process | a produced work: attribution |

The GeoJSON names the licence and links to it in its `attribution` member, as data has to. The SVG
and the PNG show `© OpenStreetMap contributors` and the full address of
<https://www.openstreetmap.org/copyright> in their bottom right corner, where no label is allowed to
cover them; a picture holds no link, so the address is written out, and the SVG links it as well. A
map is a Produced Work, and a credit hidden in a file's metadata would be gone the moment it became a
picture.

The GeoJSON and the SVG are byte-deterministic given the same harvest. **A PNG is not**, because the
glyphs come from whatever fonts the machine has. That is equally true of a browser. When a run has
to be defensible afterwards, keep the harvest and the SVG.

**No map and no harvest is ever written over.** When the name asked for is taken, today's date is
added, then a counter, `abidjan-2026-10-19.svg`, and the report names the file used; `-` writes to
stdout instead. The report and the progress go to stderr, where `-q` leaves nothing but errors and
the attribution. A run exits 0 when it succeeds, 1 when it fails and 2 when it was asked for wrongly.

## Themes are files, not code

```sh
planetrs-mapclipper themes                    # atlas, tonal, linework
planetrs-mapclipper render … --theme ./mine.toml
```

A theme names a background, an ink, a draw order and what each category is painted with:

```toml
name = "linework"
background = "#f4f2ec"
ink = "#33353a"
order = ["green", "water", "sea", "road-minor", "road-major"]

[water]
fill = "#aad2e5"

[road-major]
stroke = "#ffffff"
width = 2.2
```

A category with a colour but no place in `order` is not drawn, which is how a theme turns something
off without deleting it. The reader is strict: an unknown key or an unknown category is an error
naming its own line, because a theme that silently styles nothing produces a blank map rather than a
complaint.

The categories are fixed by the tool, so a theme decides what a quay looks like and never what counts
as one.

## Names

```sh
planetrs-mapclipper render abidjan.mapclip … --labels settlements
planetrs-mapclipper render abidjan.mapclip … --labels settlements,water
planetrs-mapclipper render abidjan.mapclip … --labels all
```

Three classes: `settlements` (towns, villages, hamlets), `water` (ponds, lakes, rivers, brooks) and
`areas` (woods, parks, industrial estates). Off by default.

**Names always go into the GeoJSON, whatever `--labels` says.** They sit in a `labels` member beside
`features`, each with its text, class, rank and position. A reader drawing its own map wants to
place them its own way rather than take ours, and RFC 7946 allows the foreign member.

What `--labels` decides is only what the *picture* carries, because placement is a pixel problem and
pixels only exist at drawing time. The scene ranks every name it found; the renderer then walks them
strongest first and draws each one whose box is still free. A village therefore beats a brook for a
crowded corner, a name that would run off the canvas is dropped rather than clipped, and nothing is
ever drawn over the marker.

Two things this deliberately does not do. **Roads are not labelled**, because text following a
curving path at intervals is a different and much larger job, and doing it badly looks worse than
not doing it. And a watercourse gets **one** label, not one per way: OSM splits a brook wherever
anything about it changes, so one brook can arrive as a dozen separate ways and drawing each would
stack the same word down its length.

Sizes and colours come from the theme:

```toml
[label-settlement]
size = 13
colour = "#3a3a3c"

[label-water]
size = 10
colour = "#5b8ca8"
```

A theme that states none of this still labels, using the built-in defaults, and `halo` falls back to
the theme's background so a name stays readable over water.

## Two things worth knowing about the data

**OSM does not store the sea.** It stores `natural=coastline` as open, directed ways with land on
the left and water on the right. A frame cut out of that has a few line fragments and nothing saying
which side is wet. Recovering the surface means closing those fragments against the frame's own
edges, walking the boundary **clockwise**. The other way yields the land, which is the same polygon
inverted and looks entirely plausible until you notice the sea is inland.

**A stream is a line, not a surface.** OSM stores `waterway=river|stream|canal` as a centreline a
couple of metres wide, and only `waterway=dock|riverbank` and `natural=water` as surfaces. They are
separate categories here (`waterway` and `water`) so a theme can stroke one and fill the other.
Conflating them, and then letting SVG close an open path implicitly before filling it, draws a brook
as a wedge following its own course and makes it read wider than the pond beside it.

**Detail below a pixel is not free.** One frame's GeoJSON went from 38 MB to 485 KB with no visible
difference, because 153 025 of 158 307 features were buildings under a pixel across. Simplification
is Douglas–Peucker at half a pixel, and the threshold is in pixels rather than metres because the
frame is what decides what can be seen.

## Framing

Ground size is asked for in kilometres, never in degrees. A degree of longitude is `1/cos(latitude)`
shorter than a degree of latitude, so a box in raw degrees is 1.62 times too wide at Rotterdam and
2.88 times too wide at Tromsø.

`width-km ÷ width-px` gives metres per pixel, which is the only number that compares two renders
directly. **The frame's aspect ratio and the canvas's must match** or the map is stretched: 29 km on
1000 px wants 16.53 km on 570, not 17.1. `planetrs-mapclipper` measures the disagreement and says so.

`--margin` is how far beyond the frame to collect, as a share of it. A way is kept when one of its
nodes falls in the collected box, so drawing it needs the vertices just outside the frame too, or a
quay running off the edge stops dead at the boundary instead of leaving the picture.

## The id index

Five passes read the planet, and the last two fetch a multipolygon's member ways and then their
nodes by id. Those used to be full scans of the way and node sections: half the I/O of the whole
harvest, spent fetching a few thousand entities.

The planet declares `Sort.Type_then_ID`, so ids ascend within a section and a blob's span never
overlaps its neighbour's. Passes one and two inflate those sections anyway, so recording each blob's
first and last id costs nothing, and the passes that follow read a handful of blobs instead of
eighteen thousand. The index lives for one harvest and is never written down: the passes that build
it read those sections in any case, so a copy kept on disk would spare no blob.

The sort order is **checked against the file's own header**, not assumed. A file that does not
declare it is refused rather than bisected wrongly.

## Installing

```sh
cargo install planetrs-mapclipper --features cli
```

`cargo install --git https://github.com/maeevick/planetrs planetrs-mapclipper --features cli` builds
the latest commit instead.

Release archives for macOS and Linux are on the
[releases page](https://github.com/maeevick/planetrs/releases), and a planet or a smaller extract
comes from `planetrs-geocoder download` or any OpenStreetMap mirror. A harvest needs a file that
declares `Sort.Type_then_ID`, as the planet and Geofabrik's extracts do.

Without its `cli` feature the crate is the library alone, clap and the rest of the command left out:
a harvest is `harvest::passes::harvest_area`, a scene is `scene::build`, and each output is a
function of a scene. `--no-default-features` drops the PNG output and resvg too. The PBF reader
under `crates/` pulls nothing beyond `flate2`, so embedding it costs nothing.

## Data and licence

The code is MIT ([LICENSE](https://github.com/maeevick/planetrs/blob/main/LICENSE)). OpenStreetMap is
© OpenStreetMap contributors and available under the Open Data Commons Open Database License (ODbL)
by the OpenStreetMap Foundation (OSMF): <https://www.openstreetmap.org/copyright>. A harvest and a
GeoJSON are Derivative Databases, shared under the ODbL with the notice they carry; an SVG or a PNG is
a Produced Work and keeps its credit, and the copyright page's address, visible.
