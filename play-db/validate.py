#!/usr/bin/env python3
"""Validate play-db entries against the JSON schema."""

import json
import sys
from pathlib import Path

try:
    import jsonschema
    from jsonschema import validate, ValidationError
except ImportError:
    print("ERROR: jsonschema not installed. Run: pip install jsonschema")
    sys.exit(1)

try:
    import tomli
except ImportError:
    # Fallback to tomllib (Python 3.11+)
    import tomllib as tomli


def load_schema(schema_path: Path) -> dict:
    """Load the JSON schema."""
    with open(schema_path, "r") as f:
        return json.load(f)


def validate_entry(entry_path: Path, schema: dict) -> list[str]:
    """Validate a single entry against the schema. Returns list of errors."""
    errors = []
    try:
        with open(entry_path, "rb") as f:
            entry = tomli.load(f)
        # Convert to JSON-compatible dict for schema validation
        validate(instance=entry, schema=schema)
        print(f"  OK: {entry_path}")
    except ValidationError as e:
        errors.append(f"{entry_path}: {e.message}")
        print(f"  FAIL: {entry_path}: {e.message}")
    except Exception as e:
        errors.append(f"{entry_path}: {e}")
        print(f"  ERROR: {entry_path}: {e}")
    return errors


def validate_runners(runners_path: Path) -> list[str]:
    """Validate runners.toml format. Returns list of errors."""
    errors = []
    try:
        with open(runners_path, "rb") as f:
            data = tomli.load(f)

        if "runners" not in data:
            errors.append(f"{runners_path}: missing 'runners' array")
            return errors

        for i, runner in enumerate(data["runners"]):
            required = ["runner_type", "version", "url", "sha512"]
            for field in required:
                if field not in runner:
                    errors.append(f"{runners_path}: runner[{i}] missing '{field}'")

            # Check SHA512 format (128 hex chars)
            sha512 = runner.get("sha512", "")
            if sha512.startswith("PLACEHOLDER_"):
                # Placeholder checksums are allowed but warned
                print(f"  WARN: {runners_path}: runner[{i}] has placeholder SHA512")
            elif len(sha512) != 128 or not all(c in "0123456789abcdefABCDEF" for c in sha512):
                errors.append(f"{runners_path}: runner[{i}] invalid SHA512 format (expected 128 hex chars)")

        print(f"  OK: {runners_path} ({len(data['runners'])} runners)")
    except Exception as e:
        errors.append(f"{runners_path}: {e}")
        print(f"  ERROR: {runners_path}: {e}")
    return errors


def main():
    """Main validation entry point."""
    repo_root = Path(__file__).parent
    schema_path = repo_root / "entry.schema.json"
    runners_path = repo_root / "runners.toml"
    entries_dir = repo_root / ".entries"

    all_errors = []

    print("Validating play-db...")

    # Load schema
    print("\n[1/3] Loading schema...")
    try:
        schema = load_schema(schema_path)
        print(f"  OK: {schema_path}")
    except Exception as e:
        print(f"  ERROR: {schema_path}: {e}")
        sys.exit(1)

    # Validate runners.toml
    print("\n[2/3] Validating runners.toml...")
    all_errors.extend(validate_runners(runners_path))

    # Validate all entries
    print("\n[3/3] Validating entries...")
    if entries_dir.exists():
        for entry_path in entries_dir.glob("*/*/default.toml"):
            all_errors.extend(validate_entry(entry_path, schema))
    else:
        print("  No entries directory found (skipping)")

    # Summary
    print("\n" + "=" * 50)
    if all_errors:
        print(f"FAILED: {len(all_errors)} error(s)")
        for err in all_errors:
            print(f"  - {err}")
        sys.exit(1)
    else:
        print("PASSED: All validations successful")
        sys.exit(0)


if __name__ == "__main__":
    main()
