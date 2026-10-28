# Contributing

Thank you for wanting to help. It doubles the size of the team, and maeevick is mostly relieved.

## Before you start

Open an issue first for anything larger than a fix, so that nobody spends a weekend on a change that
cannot be merged. A bug report is most useful with the command, the site or the area, what came out,
and what you expected. A geocoding problem is best shown with `planetrs-geocoder trace "…"`, which
prints every decision the lookup takes.

Never put somebody else's private data in an issue, a test or a commit: no customer files, no real
people's addresses. Invent the site, or use a public landmark.

## Trying it on another territory

planetrs has been tested on France and Monaco, and nowhere else. Finding out how it does on your own
patch of the planet is the most useful thing anyone can do for it, and it takes no Rust at all:

1. Get the data. `download` fetches the whole planet; an extract of your country from
   [Geofabrik](https://download.geofabrik.de) is lighter, through `download --from` and its address.
2. Index it: `planetrs-geocoder index --countries XX` and `planetrs-mapclipper index --countries XX`
   with your country's ISO code, and `--planet PATH` when it is an extract.
3. Geocode addresses that are public: town halls, schools, shops, landmarks. Your country's open
   address register, if it has one, makes a good yardstick. Note how the precisions come out, and run
   `planetrs-geocoder trace` on the misses to see where the reading went wrong.
4. Cut a few maps, and for a box or two set `planetrs-mapclipper harvest` from the dataset against
   `harvest --planet` from the file itself: `cmp` should find nothing between them.
5. Open a territory report, the issue form of that name: the territory, the file and its date, what
   you ran, how many addresses, what came out, and a few examples.

"It does not work here, and this is why" is as welcome as "it works". A pull request teaching the
reading rules another country's addresses is welcome too, with tests on public addresses. All of this
is voluntary, as everywhere in open source: there is no bounty and no payment, now or later.

## The house rules

- **Tests first.** Every change starts with a failing test, then the code that makes it pass, then a
  tidy-up. Tests are named `given_…_when_…_then_…`, test one behaviour each, and use fakes rather
  than mocks.
- **Names over comments.** Code explains itself through its names. Doc comments say why something is
  the way it is; comments inside a function are a sign that a name is missing.
- **`make check` passes**: formatting, clippy at `-D warnings`, then every test.
- **Rust 1.88** is the oldest supported toolchain; `make msrv` checks it. Raising it is a change of its
  own, called out in the changelog.
- **System tests** read a real planet and run on request: `PLANET=/path/to/planet.osm.pbf make
  test-system`. They are `#[ignore]` so that `make check` never needs ninety gigabytes.

## Commits

Conventional commits, a title and nothing else:

```text
feat(geocoder): read a road number written after a door
fix(mapclipper): keep the credit clear of the marker
docs: explain the never-overwrite rule
```

The types are `feat`, `fix`, `docs`, `test`, `build`, `ci`, `chore`, `refactor` and `perf`; the scope,
when there is one, is the crate's short name (`pbf`, `geo`, `geocoder`, `cli`, `mapclipper`). A commit
that needs a paragraph to explain itself is two commits. Pull requests are squashed, so the title of
the pull request becomes the commit.

## Data

Everything derived from the planet follows the ODbL; see the
[README](https://github.com/maeevick/planetrs#data-and-licence). Contributions to the code are
accepted under the MIT licence of the repository.
