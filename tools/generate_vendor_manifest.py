"""Generate the public jgenesis provenance manifest from the pinned upstream tree.

    python tools/generate_vendor_manifest.py [--repo DIR]

The upstream URL and revision are authoritative constants in audit_public.py. The
verified Git tree is cached under the repository's ignored .cache directory. Only
paths, Git blob ids and modification flags are written, never upstream file bytes.
Only the current HEAD/index/worktree closure is considered, never unrelated refs.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from audit_public import AuditError, ROOT, VENDOR_MANIFEST, generate_vendor_manifest


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=ROOT)
    args = parser.parse_args(argv)
    try:
        document = generate_vendor_manifest(args.repo)
        path = args.repo.resolve() / VENDOR_MANIFEST
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    except (AuditError, OSError, ValueError) as error:
        print(f"generate_vendor_manifest: ERROR: {error}", file=sys.stderr)
        return 1
    modified = sum(entry["modified"] for entry in document["files"].values())
    print(f"generate_vendor_manifest: wrote {len(document['files'])} paths; {modified} modified/local")
    return 0


if __name__ == "__main__":
    sys.exit(main())
