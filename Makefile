PLANET ?= $(or $(lastword $(sort $(wildcard planet-*.osm.pbf))),planet-latest.osm.pbf)
WORK   ?= .work

.PHONY: help build release fmt lint test test-unit test-integrated test-system test-all check msrv doc deny package index map clean

help:
	@grep -E '^[a-z-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

build: ## compile every crate
	cargo build --workspace

release: ## compile every crate with optimisations
	cargo build --workspace --release

fmt: ## format every crate
	cargo fmt --all

lint: ## refuse any warning
	cargo clippy --workspace --all-targets -- -D warnings

test-unit: ## pure logic, no filesystem
	cargo test --workspace --lib

test-integrated: ## real formats on the filesystem, no live services
	cargo test --workspace --tests

test-system: ## against a real planet file and the network, slowest: PLANET=/path/to/planet.osm.pbf
	PLANET=$(abspath $(PLANET)) cargo test --workspace --release -- --ignored

test: test-unit test-integrated ## unit then integrated, fastest first

test-all: test-unit test-integrated test-system ## every tier, fastest first

check: fmt lint test ## what has to pass before anything is done

msrv: ## the oldest toolchain supported, Rust 1.88
	cargo +1.88 check --workspace --all-targets

doc: ## the API documentation, refusing a broken link
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

deny: ## licences, advisories and sources of every dependency
	cargo deny check

package: ## what crates.io would receive, built from the packaged sources
	cargo publish --workspace --dry-run --allow-dirty

index: release ## build the geocoder's index from PLANET into the data directory
	./target/release/planetrs-geocoder index --planet $(PLANET)

map: release ## cut the demo map out of PLANET, harvest kept for re-rendering
	mkdir -p $(WORK)
	./target/release/planetrs-mapclipper clip $(PLANET) \
	  --lat 5.269 --lon -4.012 --width-km 29 --height-km 16.53 --margin 0.10 \
	  --width-px 1000 --height-px 570 --theme linework \
	  --marker name --name "en=Port of Abidjan" --poi-lat 5.27 --poi-lon -4.01 \
	  --keep-harvest $(WORK)/abidjan.mapclip --png $(WORK)/abidjan.png

clean:
	cargo clean
