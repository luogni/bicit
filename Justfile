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

# Run against the sample GPX in `test/t1.gpx`.
# Usage examples:
#   just run-sample
#   just run-sample templates/dev3.svg out
run-sample template="templates/dev.svg" outbase="out": (run-gpx "test/t1.gpx" template outbase)

# Run with an arbitrary GPX file.
# Example:
#   just run-gpx path/to/ride.gpx templates/dev.svg myride
run-gpx datafile template="templates/dev.svg" outbase="out":
	cargo run -- --datafile "{{datafile}}" --template {{template}} --outfile {{outbase}}

# Render *all* templates in `templates/` for quick visual checks.
# Example:
#   just render-all
#   just render-all test/t1.gpx out
render-all datafile="test/t1.gpx" outdir="out":
	mkdir -p {{outdir}}
	for t in templates/*.svg; do name=$(basename "$t" .svg); cargo run -- --datafile "{{datafile}}" --template "$t" --outfile "{{outdir}}/$name"; done
