"""The canonical JSON schema version these scripts read (ADR 0017). A test compares it with Rust's SCHEMA_VERSION."""

SUPPORTED_SCHEMA_VERSION = 7


def check(document, who):
    """Returns `document` once its `schemaVersion` is the supported one and stops the script otherwise. Output without a
    version predates it and has version 1's shape."""
    version = document.get('schemaVersion', 1)
    if version != SUPPORTED_SCHEMA_VERSION:
        raise SystemExit(f'{who}: the caliper output has schema version {version}, and this script reads version '
                         f'{SUPPORTED_SCHEMA_VERSION}. Run it with a caliper-docgen binary of the same release.')
    return document
