# play-db

Community database for game compatibility entries and runner manifests.

## Structure

```
play-db/
.entries/
    {hash_prefix}/
        {exe_hash}/
            default.toml
runners.toml
entry.schema.json
validate.py
.github/
    workflows/
        validate.yml
```

## Usage

This repository is cloned to `~/.local/share/play/db/` at runtime by the `play` CLI.

### Adding a Game Entry

1. Compute the SHA256 hash of the game executable
2. Create directory `.entries/{hash[0:2]}/{hash}/default.toml`
3. Fill in the entry fields (see `entry.schema.json`)
4. Run `python validate.py` to validate
5. Submit a PR

### Updating Runners

Edit `runners.toml` to add new GE-Proton releases with SHA256 checksums from the official release.

## Validation

All entries are validated against `entry.schema.json` in CI.
