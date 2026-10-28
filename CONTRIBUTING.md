# Contributing to Helix

Thanks for your interest. Helix is a Rust workspace (MSRV 1.75) plus a small Python-validated data repo under `play-db/`.

## Dev setup

```bash
rustup toolchain install 1.75.0 --component clippy rustfmt
rustup default 1.75.0
cargo build --workspace
cargo test --workspace
cd play-db
pip install jsonschema tomli
python3 validate.py
```

## Commit style

```
<type>(<scope>): <summary> [helix-polish #<N>]
```

Types: `fix`, `feat`, `refactor`, `chore`, `docs`, `test`.

## PR checklist

- [ ] Gate green on MSRV 1.75.
- [ ] No `.unwrap()` / `.expect()` outside `#[cfg(test)]`.
- [ ] No `shell=true` / string-interpolated commands.
- [ ] New tweak logic lives in a `Tweak` struct, not in an agent.
- [ ] `/proc` / `/sys` access goes through injectable roots.
- [ ] `play-db` entries remain pure data.
