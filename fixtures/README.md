# Fixtures

Captured-from-real-device byte fixtures used as the oracle for the
Rust codec. See `REGEN.md` for the regeneration recipe.

## Status

- `minimal/` — empty (capture pending; maintainer has the BluePill)
- `wide_coverage/` — empty (capture pending; needs hand-tuned config)
- `params_stream/` — empty (capture pending)

Once captures land, this README gets a summary of each fixture's
device source, capture date, and `FIRMWARE_VERSION`.

## What lives here vs. in code

Fixture **bytes** (`*.bin`) live here, treated as binary blobs by git.
Fixture **interpretation** (`expected.ron` files) also lives here as
hand-authored YAML/RON describing what the bytes mean.

The Rust codec tests in `crates/freejoyx-core/tests/` load these
fixtures via relative path. Do not move this directory without
updating the test loader.
