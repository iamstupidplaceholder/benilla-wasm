# Benilla's `rilua` fork

Upstream: `rilua` 0.1.24, MIT OR Apache-2.0, from the crates.io source package.
The workspace selects this copy with `[patch.crates-io]`, but all behavior
changes are gated behind the off-by-default `wow_112` feature. The
`benilla-lua` adapter enables it by default and forwards its own `wow_112`
feature to the VM. A future client build can disable that adapter default
without editing the fork; without the feature, the VM retains stock Lua 5.1.1
behavior. Do not update the upstream base and Vanilla compatibility edits in
the same change: each observed client-dialect difference should stay reviewable.

## Why this fork exists

WoW 1.12.1 does not run stock Lua 5.1. Its shipped scripts and installed addons
exercise eight measured differences, documented in `../lua-src/BENILLA.md`.
`rilua` is a good browser VM baseline, but the L1 browser probe on 2026-09-21
reported `6/6` host capabilities and `0/8` Vanilla dialect checks. This is a
semantic mismatch, not a WASM or embedding failure.

The goal is a **WoW 1.12 dialect mode**, not a wholesale Lua 5.0 downgrade and
not a source-text rewrite. The client accepts some 5.0 constructs that 5.1
removed and rejects some 5.1 constructs that addons use for version detection.

## Browser-verified first slice

- `src/compiler/parser.rs`: reject vararg expressions, unary `#` and binary
  `%`, while preserving the existing internal AST/bytecode support; permit
  surplus semicolons inside table constructors.
- `src/compiler/lexer.rs`: restore nesting of level-zero `[[...]]` long
  strings/comments. Explicit `[=[...]=]` levels retain their 5.1 behavior.
- `src/vm/execute.rs`: if a generic-for generator is a table, move it to the
  iterator state and replace the generator with the current raw global `next`
  before the first call. This mirrors the project's byte-verified native patch;
  it does not rely on `__call` and does not rewrite the source.

The browser L1 report after this slice was `6/6` host capabilities and `6/8`
Vanilla-dialect checks. Only computed-key table order and error quote spelling
remained in that probe.

## Browser-verified second slice

- `src/compiler/codegen.rs`: computed `[key] = value` constructor fields do not
  increase the `NEWTABLE` hash-size hint in `wow_112` mode. This matches the
  native Lua 5.0 parser change and lets dense numeric keys rehash into the
  array part, where `next` visits them in ascending order.
- `src/vm/execute.rs`: named runtime type errors use Lua 5.0's `` `name' ``
  spelling in `wow_112` mode. This fixes the measured `client-error-quotes`
  probe, not every possible diagnostic source in the VM and standard library.

The existing 5.1 three-slot generic-for register layout remains. Lua 5.0 used
two hidden slots, so assigning to the first loop variable inside a loop can
still differ; the native fork documents this measured divergence too. Do not
claim full equivalence from the simple L1 probe.

Rejecting `...` expressions also does not yet restore Lua 5.0's implicit
`arg` table in vararg functions. That needs a separate runtime compatibility
check before loading the full addon corpus.

The browser L1 report passed `8/8` measured behaviors and `6/6` host
capabilities after this slice. This is a checkpoint, not proof of complete
client-dialect or addon-corpus equivalence.

The production native client continues to use `mlua` plus Benilla's separate
`lua-src` fork. This fork serves only the browser-facing `benilla-lua` seam
until the full UI host API and addon corpus have been validated.

The feature is currently compile-time because this browser port targets one
client version per build. If the application must support multiple WoW script
versions in one process, move the dialect choice into per-VM `Lua` state and
propagate it through compiler and VM entry points; do not use Cargo features
for that case because features are unified across a dependency graph.
