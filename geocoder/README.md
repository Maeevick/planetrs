# planetrs-geocoder

A coordinate for a place or a postal address, from OpenStreetMap data alone.

No API, no rate limit, no terms of use on what you do with the answer. The planet file already holds
every address point, shop and administrative boundary; what it does not hold is any notion of what a
badly typed address meant. So the work is all in deciding what to accept as a match, and in saying
how much had to be forgiven to reach it. Veil, who hunts the street an address meant for the
[planetrs](https://github.com/maeevick/planetrs) goblins, puts it more briefly: shhhhh, and no
guessing.

## The principle

**Every answer says what kind of answer it is.** A house number on the right street and the middle
of a town are both "found", and only one of them is an address. Reporting a town centre as a bare
coordinate is the most dangerous thing a geocoder can do, because it looks exactly like an address.

**It refuses rather than guesses.** When nothing better than a commune's centre was found, the answer
says `Commune`, and the command line asks you to look at it before using it.

| precision            | where the coordinate stands                                                |
| -------------------- | -------------------------------------------------------------------------- |
| `HouseNumber`        | the asked door of the asked street                                         |
| `NearestHouseNumber` | the recorded door of the asked street closest to the asked number          |
| `Shop`               | a shop of the site's label on the asked street, or alone of its label in the commune |
| `Street`             | the middle of the asked street, or of the named hamlet                     |
| `Commune`            | the commune's centre                                                       |
| `NotFound`           | not even the commune: latitude and longitude are left empty                |

## Use it

```sh
planetrs-geocoder download             # the latest planet, about 90 GB, resumable, md5-checked
planetrs-geocoder index                # France and Monaco: four minutes and 20 GB of memory
planetrs-geocoder "Acme;8 Route des Tilleuls;89100;Sens;France"
planetrs-geocoder -f sites.csv -o answers.csv
planetrs-geocoder trace "Acme;8 Route des Tilleuls;89100;Sens;France"
planetrs-geocoder status
```

A site is five fields, `label;address;postal code;city;country`. A site file starts with that header,
in any order and any case, is UTF-8, and quotes a field holding a semicolon, a double quote or a line
break. Answers repeat the five fields exactly as written, then `lat;lon;precision`:

```text
label;address;postal code;city;country;lat;lon;precision
Acme;8 Route des Tilleuls;89100;Sens;France;48.200000;3.265200;HouseNumber
Bazar;1 Rue Haute;99999;Nulle Part;France;;;NotFound
```

One site prints to stdout. A file writes to `output.csv`, or the name `-o` gives, and **never writes
over a file**: when the name is taken it adds today's date, then a counter, and says which name it
used. `-f -` reads stdin and `-o -` writes to stdout. Progress, counts and the attribution go to
stderr, so the answers can be piped, and `-q` leaves nothing there but errors and the attribution.

| exit code | means |
| --------- | ----- |
| 0         | done, `NotFound` rows included |
| 1         | something failed: no dataset, a file that cannot be read or written |
| 2         | the command or the site file is malformed, and nothing was geocoded |

Planets, the dataset and the cache of a remote dataset live in a data directory: `--data-dir`, else
`PLANETRS_DATA_DIR`, else `$XDG_DATA_HOME/planetrs`, else `~/.local/share/planetrs`. `--dataset
DIR|URL`, else `PLANETRS_DATASET`, reads a dataset from another directory or from a web address;
`index` builds into a directory only.

## As a library

```rust
use planetrs_geocoder::{Geocoder, Site, refusal_of};

let mut geocoder = Geocoder::open("dataset")?;
let site = Site {
    label: "Acme".into(),
    address: "8 Route des Tilleuls".into(),
    postcode: "89100".into(),
    town: "Sens".into(),
    country: "France".into(),
};
let answer = geocoder.locate(&site)?;
if let Some(answer) = &answer {
    println!("{:.6};{:.6};{}", answer.latitude(), answer.longitude(), answer.precision);
}
if let Some(reason) = refusal_of(&site, answer.as_ref()) {
    eprintln!("look before using it: {reason}");
}
```

`planetrs_geocoder::build_index` builds a dataset from a planet file. `Geocoder::open` reads a
dataset's directory, and `Geocoder::over` any dataset `planetrs-dataset` reaches, over HTTP
included. Opening reads the gazetteer alone. A lookup then reads its country's word counts once and
its commune with the communes stored beside it, so a first lookup in France takes well under a
second and about 500 MB, and a later one in a commune already read touches no disk.
The API documentation is on [docs.rs](https://docs.rs/planetrs-geocoder).

## Why the commune is settled first

The commune is resolved from `admin_level=8` boundary relations, which carry the name, the official
reference and the postcode. Matching a postcode and a town name against those settles almost every
site to exactly one commune, in a pass over the whole planet.

Settling it first does two things. It makes a whole class of failure impossible rather than unlikely:
a search for `97400 Saint-Denis` cannot answer from the Saint-Denis north of Paris, because the
commune was decided before any street was looked at. And it shrinks the candidate pool from every street in the country to the few
hundred of one commune, which is what makes character-level comparison affordable at all.

## How a site is read

Parentheses are removed first; then the address is read in every way a spaced dash or a designator
allows, and the reading that matches best answers.

**Parentheses.** Anything between parentheses, in any of the five fields, is removed:
`17 rue des Écoles (LE BOURG)` reads `17 rue des Écoles`.

**Every reading, exact first.** The address is read several ways: whole, the text before its first
spaced dash (a dash with a space on at least one side), the text after it, each split into parts at
commas and slashes, and each of those parts again without the designators that open it. When any
reading matches a street exactly, it is kept. A matched door beats a nearest door, which beats a
street, and among equals the order above wins.

| address                                   | answered on                                  |
| ----------------------------------------- | -------------------------------------------- |
| `150 Avenue de l'Europe - Robert Schuman` | the whole field: a street of that name       |
| `CC Cap Horizon - Rue Lavandière`         | `Rue Lavandière`, after the dash             |
| `ZAC St Martin`                           | `ZAC St Martin`, a road named after its zone |
| `C.C. La Pinsonnière`                     | `La Pinsonnière`, without its designator     |

An exact name is not worth a door already found, though. If a looser reading places the site closer,
a door against a street's middle, the looser reading answers: in `2 Bis Rue Mollière/74 Vieux Port`
the door on `Rue Molière` beats the quarter of `Vieux Port`.

**Otherwise, the loose reading.** When no reading matches exactly, the field is read as it was
written, dash included: its parts are searched as they are, then without their designators, each as
loosely as the street tiers allow. A zone's name is weaker evidence than a street written beside it,
so in `ZA du Pont Vieux/Rue Paul Cézanne` the street answers. The field is not cut at the dash here,
since every exact reading on either side of it has already been tried, and cutting would lose the
words that make `Route des Saules - Beauregard` resemble `Route de Beauregard`.

A dash between two door numbers is a range (`86 - 88 Rue Corot`), a dash inside a name has no space
(`Saint-Exupéry`), and hyphens, en dashes and em dashes follow the same rule. `et` and `and` between
two addresses, `5 rue Matisse et 19 rue Degas`, separate them as a slash does; between two doors,
`8 et 10 Route des Tilleuls`, they are a range.

**Designators** are those of the French profile: `C.C`, `CC`, `CCAL`, `C.Cial`, `CCial`, `Ctre Cial`,
`Centre Cial`, `Centre Commercial`, `Galerie Marchande`, `Galerie Commerciale`, `Espace Commercial`,
`Parc Commercial`, `Parc d'Activité(s)`, `ZA`, `ZAC`, `ZAE`, `ZI`, `ZC`, `Zone`, `Zone Artisanale`,
`Zone Industrielle`, `Zone Commerciale`, `Zone d'Activité(s)`, `Zone d'Aménagement Concerté`,
`Lieu-dit`, `LD` and `Quartier`. They are skipped one after another where they open a part,
`C.Cial Galerie Marchande Sommet` leaving `Sommet`, and kept inside a name, `12 Rue du Centre
Commercial`.

## How a street is matched

Each part is compared with the streets of the settled commune through tiers, strictest first: exact,
the same letters spaced differently, most words shared, most letters shared, and the same name with
another street type. The first tier that holds answers.

**When no tier matches any part**, the parts are tried once more against three looser rules, each
answering only when a single street of the commune fits it:

- **both ways:** every distinctive word pairs with one of the street's, forgiven a typo scaled to its
  length, the same French sound from six letters, an initial (`J` for `JEAN`), or a missing space;
  spelling is forgiven only when the street types agree;
- **asked within the street:** every asked word is in the street letter for letter and the street has
  more, `RUE MITTERRAND` for `RUE PRESIDENT MITTERRAND`, provided the asked words are rare and
  outweigh the street's extra ones;
- **street within the asked text:** every word of the street is in the text letter for letter, and
  the text has more as one block before or after it, as a centre's name or a locality does.

A street type pairs only with a street type, each rule forgives one kind of difference only, and two
fitting streets end the search. Rarity is counted over every street name the index holds. A street
these rules find at no recorded door ranks below the one shop of the label in the commune, and above
the commune's centre.

**When even those find nothing**, the address is rewritten before settling for the centre. It is
split at every separator a person uses, `/`, `,`, `-`, `et`, `and`, and each piece is tried as it is
and with each of its words left out in turn: one wrong word, `Général` for `Maréchal`, hides a street
as surely as a missing one. The narrowest answer wins, and between equals the rewriting that left out
the least informative word. Two equally good rewritings on two different streets are a guess, and the
centre stays. A rewriting must also keep enough rare words to name a street: `ROUTE SAINT`, all that
is left of `Route de Saint Vigor` without `Vigor`, names none.

**Road numbers.** The index files every road's `ref` as a name of its own, `D620` for `ref=D 620`,
and a road known only by its number is filed under it. In a site, `RD 620`, `RD620`, `R.D. 620`,
`CD 620`, `D 620` and `Route Départementale 620`, and `RN 7` or `Route Nationale 7` for a national
road, all read as that number, wherever in a part they stand: `118 RD 712 Les Ponts Neufs` reads as
door 118 of `D712`. A road's number is weak evidence, since a departmental road runs for kilometres,
so it is the last answer tried before the centre, and it must match exactly: `D620` is not `D620A`.

## How a shop answers

The label answers when nothing else about the site is in doubt: a shop of that label standing on the
asked street, by its own `addr:street`, or the only shop of that label in the commune.

Shops of one label within 150 m of each other count as one. OpenStreetMap often maps one shop twice,
as a point and as its building, and files a shop under both a merged commune and its former town;
150 m is the footprint of a large supermarket, so copies fall inside it while two branches in one
town usually do not. Shops of one label further apart, but all within a kilometre of their middle,
answer with that middle rather than the commune's centre. Shops spread wider than that are different
places, and the label does not choose between them.

## When the centre is an answer

A site placed at its commune's centre is refused, because a centre written as a coordinate looks like
an address and is not one. The smallest communes are spared: the index measures every commune's area
from its outline, holes subtracted, and in a commune of at most 3.5 km², a disc about a kilometre
across, the centre is within about a kilometre of the site wherever it stands. The answer still says
`Commune`; it is only no longer refused.

## Why a kilometre

The geocoder was built to place sites on a weather model's grid, whose cells are about a kilometre
across. Within a cell, a door and a street's middle are the same answer, which is why the rules that
trade precision for coverage, a shop's middle or a small commune's centre, stop at a kilometre and go
no further.

## How it got here

The rules arrived one at a time, each measured against reference geocoders on real site files before
it was kept, and the code still calls them by the step that brought them:

| step | what it added |
| ---- | ------------- |
| 1    | the town name and the country, against every commune of the planet |
| 2    | the postcode, which picks the commune out of those answering to the name |
| 3    | streets, doors and hamlets of the settled commune |
| 4    | refusing an answer that only reached the commune's centre |
| 5    | the former town a postcode names, first, inside a merged commune |
| 6    | the nearest recorded door when the asked one is missing |
| 7    | every door of a field: `5-7`, `5/7`, `8 et 10`, `22 à 28`, `30 Bis à 30 Ter` |
| 8    | parentheses ignored |
| 9    | the label, against the shops of the commune |
| 10   | the nearest door on the asked side of the street |
| 11   | the spaced dash as a note, not a part of the street |
| 12   | designators skipped where they open a part |
| 13   | every reading tried, exact matches first |
| 14   | the loose reading keeps the dash |
| 15   | the three rules for the one street a field can mean |
| 16   | `et` and `and`, and rewriting an address that found nothing |
| 17   | a small commune's centre as an answer |
| 18   | road numbers |
| 19   | the middle of a label's shops within a kilometre |

## Limits

- The reading rules are French. Streets of other countries can be indexed with `--countries`, and are
  read with the same rules, which suit them less.
- The index is built on one machine from one planet file; a remote index is not supported yet.
- macOS and Linux only, for now.

## Data and licence

The code is MIT ([LICENSE](https://github.com/maeevick/planetrs/blob/main/LICENSE)).
OpenStreetMap is © OpenStreetMap contributors and available under the Open Data Commons Open Database
License (ODbL) by the OpenStreetMap Foundation (OSMF): <https://www.openstreetmap.org/copyright>. An
index is a Derivative Database, and every one of its files names the
[licence](https://opendatacommons.org/licenses/odbl/1-0/) and links to it; a geocoding result, one at
a time, is an insubstantial extract, and wherever it is shown, OpenStreetMap is credited. See the
[planetrs README](https://github.com/maeevick/planetrs#data-and-licence) for the whole of it.
