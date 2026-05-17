# Vendored firmware headers

`common_defines.h` and `common_types.h` are copied verbatim from
[`anpeaco/FreeJoyX`](https://github.com/anpeaco/FreeJoyX)'s
`application/Inc/`. They are the **canonical source** for the
on-the-wire format the Rust codec implements.

## Do not edit by hand

These files are kept in lockstep with the firmware repo by CI
(`.github/workflows/header-sync.yml`). Editing them in place will:

1. Pass locally (the codec will build against the local edits).
2. Fail CI on push (the workflow clones FreeJoyX and normalized-diffs).
3. Most importantly: the codec will diverge from what the device
   actually produces on the wire.

## To update

When the firmware's headers change:

```sh
cp ../FreeJoyX/application/Inc/common_defines.h vendored/common_defines.h
cp ../FreeJoyX/application/Inc/common_types.h   vendored/common_types.h
git diff vendored/    # review what changed
```

Then update the Rust codec in `crates/freejoyx-core/src/wire/` to
match the new format, regenerate fixtures (per `fixtures/REGEN.md`),
and commit all three together.

## Current pin

See `docs/ported-from.md` for the FreeJoyX commit these copies match
at bootstrap. CI catches divergence after that point.
