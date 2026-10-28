PLANET ?= $(or $(lastword $(sort $(wildcard planet-*.osm.pbf))),planet-latest.osm.pbf)
WORK   ?= .work

.PHONY: help build release fmt lint test test-unit test-integrated test-alone library-alone test-system test-all check msrv doc deny package index map clean

help:
	@grep -E '^[a-z-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

build: ## compile every crate, with all its features
	cargo build --workspace --all-features

release: ## compile every crate with all its features and optimisations
	cargo build --workspace --all-features --release

fmt: ## format every crate
	cargo fmt --all

lint: ## refuse any warning, every crate with all its features
	cargo clippy --workspace --all-targets --all-features -- -D warnings

test-unit: ## pure logic, no filesystem
	cargo test --workspace --lib --all-features

test-integrated: ## real formats on the filesystem, no live services
	cargo test --workspace --tests --all-features

# Cargo turns a crate's features on for the whole workspace at once, so a command line can lean on a
# feature that only the other one asks for. cargo install builds each alone, and so does this.
test-alone: ## each command line's tests, built on its own as cargo install builds it
	cargo test -p planetrs-geocoder --features cli --tests
	cargo test -p planetrs-mapclipper --features cli --tests

# A library's users take nothing of what only its command needs.
library-alone: ## each library built on its own, without its command or anything only the command needs
	cargo check -p planetrs-geocoder -p planetrs-mapclipper
	! cargo tree -p planetrs-geocoder -p planetrs-mapclipper -e normal --prefix none | grep -E '^(clap|anyhow|planetrs-cli|ureq) '

test-system: ## against a real planet file and the network, slowest: PLANET=/path/to/planet.osm.pbf
	PLANET=$(abspath $(PLANET)) cargo test --workspace --all-features --release -- --ignored

test: test-unit test-integrated ## unit then integrated, fastest first

test-all: test-unit test-integrated test-system ## every tier, fastest first

check: fmt lint test test-alone library-alone ## what has to pass before anything is done

msrv: ## the oldest toolchain supported, Rust 1.88
	cargo +1.88 check --workspace --all-targets --all-features

doc: ## the API documentation, refusing a broken link
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

deny: ## licences, advisories and sources of every dependency
	cargo deny check

# Cargo takes a registry crate of a given version never to change, so a second dry run would build
# against the crates the first one packaged. Each run therefore starts from nothing.
package: ## what crates.io would receive, built from the packaged sources
	rm -rf target/package-verify $${CARGO_HOME:-$$HOME/.cargo}/registry/src/-*/planetrs-*
	CARGO_TARGET_DIR=target/package-verify cargo publish --workspace --dry-run --allow-dirty

index: release ## build the geocoder's index from PLANET into the data directory
	./target/release/planetrs-geocoder index --planet $(PLANET)

map: release ## cut the demo map out of PLANET, harvest kept for re-rendering
	mkdir -p $(WORK)
	./target/release/planetrs-mapclipper clip --planet $(PLANET) \
	  --lat 5.269 --lon -4.012 --width-km 29 --height-km 16.53 --margin 0.10 \
	  --width-px 1000 --height-px 570 --theme linework \
	  --marker name --name "en=Port of Abidjan" --poi-lat 5.27 --poi-lon -4.01 \
	  --keep-harvest $(WORK)/abidjan.mapclip --png $(WORK)/abidjan.png

clean:
	cargo clean
