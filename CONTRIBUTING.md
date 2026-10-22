# Contributing

Thank you for wanting to help. It doubles the size of the team, and maeevick is mostly relieved.

## Before you start

Open an issue first for anything larger than a fix, so that nobody spends a weekend on a change that
cannot be merged. A bug report is most useful with the command, the site or the area, what came out,
and what you expected. A geocoding problem is best shown with `planetrs-geocoder trace "…"`, which
prints every decision the lookup takes.

Never put somebody else's private data in an issue, a test or a commit: no customer files, no real
people's addresses. Invent the site, or use a public landmark.

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
