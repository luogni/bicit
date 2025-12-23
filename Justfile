# Run `just --list` to see recipes.

# Default recipe
_default:
    @just --list

build:
    cargo build

release:
    cargo build --release

test:
    cargo test

clippy:
    cargo clippy --all-targets -- -D warnings

test-all: test clippy
    cargo fmt

run-ui:
    cargo run -p bicit-ui

# just run-sample templates/dev3.svg out
run-sample template="../bicit/templates/dev.svg" outbase="out": (run-gpx "test/t1.gpx" template outbase)

# Run with an arbitrary GPX file.
# Example:

# just run-gpx path/to/ride.gpx templates/dev.svg myride
[working-directory("bicit-cli")]
run-gpx datafile template="templates/dev.svg" outbase="out":
    cargo run -p bicit-cli -- --datafile "{{ datafile }}" --template {{ template }} --outfile {{ outbase }}

# Render *all* templates in `templates/` for quick visual checks.
# Example:
#   just render-all

# just render-all test/t1.gpx out
[working-directory("bicit-cli")]
render-all datafile="test/t1.gpx" outdir="out":
    mkdir -p {{ outdir }}
    for t in templates/*.svg; do name=$(basename "$t" .svg); cargo run -p bicit-cli -- --datafile "{{ datafile }}" --template "$t" --outfile "{{ outdir }}/$name"; done
