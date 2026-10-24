#!/usr/bin/env python3
"""Generate/check full target-specific notices with pinned cargo-about; no ROM input."""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import re
import subprocess
import tarfile
import textwrap
import tomllib
import urllib.request
from html.parser import HTMLParser
from pathlib import Path
from typing import Callable

ROOT = Path(__file__).resolve().parents[1]
VERSION = "0.9.2"
TOOL_ARCHIVE = f"cargo-about-{VERSION}-x86_64-unknown-linux-musl.tar.gz"
TOOL_URL = f"https://github.com/EmbarkStudios/cargo-about/releases/download/{VERSION}/{TOOL_ARCHIVE}"
TOOL_SHA256 = "9099a59e820c38a68b9d65f300662a567d56562f9a10f6aa4c7e86c17c2566af"
RUST_LIBRARY_SHA256 = "5647be074c8edf7339fd863055923a8fc80bc5610a8d4661ec3b767b9d392c27"
GRAPHS = {
    "linux": ("tje-native", ("x86_64-unknown-linux-gnu",)),
    "windows": ("tje-native", ("x86_64-pc-windows-gnu",)),
    "macos": ("tje-native", ("x86_64-apple-darwin", "aarch64-apple-darwin")),
    "web": ("tje-web", ("wasm32-unknown-unknown",)),
}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def legal_text(text: str) -> bytes:
    """Quote full legal prose; its underlying words/punctuation remain unchanged."""
    lines = []
    for line in text.replace("\r\n", "\n").splitlines():
        lines.extend(textwrap.wrap(line.strip(), width=60, break_long_words=False,
                                   break_on_hyphens=False, replace_whitespace=False,
                                   expand_tabs=False) or [""])
    normalized = "\n".join(lines) + "\n"
    if re.findall(r"\S+", normalized) != re.findall(r"\S+", text):
        raise ValueError("legal-text reflow changed upstream words")
    return ("\n".join("> " + line for line in lines) + "\n").encode("utf-8")


def source_fingerprint(root: Path = ROOT) -> str:
    paths = [root / "Cargo.toml", root / "Cargo.lock", root / "rust-toolchain.toml",
             root / "tools/notices.toml", root / "tools/notices.py", root / "tools/font-notices.json"]
    paths += sorted((root / "crates").glob("*/Cargo.toml"))
    paths += sorted((root / "vendor/jgenesis").rglob("Cargo.toml"))
    paths += sorted((root / "web").glob("*.js"))
    paths += [root / "web/index.html", root / "web/style.css", root / "web/package.json",
              root / "crates/tje-engine-wasmgen/src/browser.js"]
    # Sort POSIX strings, not Path objects (Windows compares paths case-insensitively).
    records = sorted((path.relative_to(root).as_posix(),
                      digest(path.read_bytes().replace(b"\r\n", b"\n"))) for path in paths)
    return digest(json.dumps(records, separators=(",", ":")).encode())


def tool_path() -> Path:
    override = os.environ.get("CARGO_ABOUT")
    path = Path(override) if override else ROOT / "scratch/notice-tools" / f"cargo-about-{VERSION}"
    if not path.exists():
        if override:
            raise ValueError(f"CARGO_ABOUT does not exist: {path}")
        # The only automatic bootstrap is a checksum-pinned, user-local Linux binary.
        import platform
        if platform.system() != "Linux" or platform.machine() not in ("x86_64", "AMD64"):
            raise ValueError("generate notices on Linux x86_64, or set CARGO_ABOUT to version 0.9.2")
        with urllib.request.urlopen(TOOL_URL, timeout=60) as response:
            downloaded = response.read()
        if digest(downloaded) != TOOL_SHA256:
            raise ValueError("cargo-about archive checksum mismatch")
        with tarfile.open(fileobj=io.BytesIO(downloaded), mode="r:gz") as archive:
            members = [member for member in archive if member.isfile()
                       and Path(member.name).name == "cargo-about"]
            if len(members) != 1:
                raise ValueError("unexpected cargo-about archive layout")
            stream = archive.extractfile(members[0])
            assert stream is not None
            with stream:
                executable = stream.read()
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(executable)
        path.chmod(0o755)
    if subprocess.check_output([str(path), "--version"], text=True).strip() != f"cargo-about {VERSION}":
        raise ValueError(f"cargo-about {VERSION} is required")
    return path


def legal_files(package: dict) -> list[Path]:
    directory = Path(package["manifest_path"]).parent
    files = []
    for path in sorted(directory.rglob("*")):
        if (path.is_file() and not path.is_symlink()
                and re.match(r"^(LICEN[CS]E|COPYING|COPYRIGHT|NOTICE)(?:[._-]|$)",
                             path.name, re.I)):
            files.append(path)
    if package["name"] == "epaint_default_fonts":
        policy = json.loads(ROOT.joinpath("tools/font-notices.json").read_bytes())
        files.extend(directory / name for name in policy["files"])
    if package.get("license_file"):
        files.append(directory / package["license_file"])
    return sorted(set(files))


def runtime_legal_texts() -> dict[str, str]:
    """The rustc component includes these even with rustup's minimal profile."""
    sysroot = Path(subprocess.check_output(["rustc", "--print", "sysroot"], cwd=ROOT, text=True).strip())
    directory = sysroot / "share/doc/rust"
    raw = directory.joinpath("COPYRIGHT-library.html").read_bytes()
    if digest(raw) != RUST_LIBRARY_SHA256:
        raise ValueError("Rust standard-library copyright changed; review and update the pinned legal input")

    class LegalText(HTMLParser):
        def __init__(self) -> None:
            super().__init__()
            self.parts: list[str] = []

        def handle_data(self, data: str) -> None:
            if data.strip():
                self.parts.append(data.strip())

    parser = LegalText()
    parser.feed(raw.decode("utf-8"))
    chunks = []
    current = ""
    for part in parser.parts:
        if len((current + part).encode()) > 800_000:
            chunks.append(current)
            current = ""
        current += part + "\n"
    chunks.append(current)
    result = {f"rust/COPYRIGHT-library-part-{index + 1}.txt": content
              for index, content in enumerate(chunks)}
    result.update({f"rust/licenses/{path.name}": path.read_text(encoding="utf-8")
                   for path in sorted(directory.joinpath("licenses").glob("*.txt"))})
    return result


def generate(tool: Path) -> dict[str, bytes]:
    subprocess.run(["cargo", "fetch", "--locked"], cwd=ROOT, check=True)
    outputs = {}
    fingerprint = source_fingerprint()
    runtime = runtime_legal_texts()
    font_policy = json.loads(ROOT.joinpath("tools/font-notices.json").read_bytes())
    policy = tomllib.loads(ROOT.joinpath("tools/notices.toml").read_text(encoding="utf-8"))
    rust_version = tomllib.loads(ROOT.joinpath("rust-toolchain.toml").read_text(encoding="utf-8"))["toolchain"]["channel"]
    for graph, (package, targets) in GRAPHS.items():
        dependencies = {}
        texts = {}
        for target in targets:
            command = [str(tool), "generate", "--locked", "--offline", "--fail",
                       "--manifest-path", f"crates/{package}/Cargo.toml", "--target", target,
                       "--config", "tools/notices.toml", "--format", "json"]
            harvested = json.loads(subprocess.check_output(command, cwd=ROOT))
            tree = subprocess.check_output(
                ["cargo", "tree", "--locked", "--package", package, "--target", target,
                 "--edges", "normal,build", "--prefix", "none", "--format", "{p}"],
                cwd=ROOT, text=True)
            actual = set(re.findall(r"^(\S+) v(\S+)", tree, re.M))
            covered = {(item["package"]["name"], item["package"]["version"])
                       for item in harvested["crates"]}
            if actual != covered:
                raise ValueError(f"{target}: cargo-about graph differs from actual Cargo normal/build graph")

            def add_text(text: str) -> str:
                content = legal_text(text)
                if not content.strip() or len(content) > 999_000:
                    raise ValueError("missing or oversized third-party licence text")
                key = digest(content)
                texts[key] = content
                return key

            for item in harvested["crates"]:
                crate = item["package"]
                key = f"{crate['name']} {crate['version']}"
                record = dependencies.setdefault(key, {
                    "name": crate["name"], "version": crate["version"],
                    "declared_license": item["license"],
                    "authors": [{"name": author} for author in sorted(crate["authors"])],
                    "repository": crate["repository"], "targets": [], "selected": [], "texts": {},
                    "source_archive": (f"https://crates.io/api/v1/crates/{crate['name']}/{crate['version']}/download"
                                       if crate["source"] else None),
                })
                record["targets"].append(target)
                if crate["license"] == "MPL-2.0":
                    for source in Path(crate["manifest_path"]).parent.rglob("*"):
                        if (source.suffix in (".rs", ".c", ".h", ".cpp", ".hpp")
                                and "Incompatible With Secondary Licenses" in source.read_text(encoding="utf-8")):
                            raise ValueError(f"{key}: MPL secondary-licence exclusion needs legal review")
                directory = Path(crate["manifest_path"]).parent
                if crate["name"] == font_policy["package"]:
                    if crate["version"] != font_policy["version"]:
                        raise ValueError("new bundled font version requires licence review")
                    for name, expected in font_policy["files"].items():
                        if digest(directory.joinpath(name).read_bytes()) != expected:
                            raise ValueError(f"bundled font notice changed: {name}")
                    record["selected"].extend(font_policy["licenses"])
                for path in legal_files(crate):
                    record["texts"][path.relative_to(directory).as_posix()] = add_text(
                        path.read_text(encoding="utf-8"))
            for license_record in harvested["licenses"]:
                text_key = add_text(license_record["text"])
                for user in license_record["used_by"]:
                    crate = user["crate"]
                    record = dependencies[f"{crate['name']} {crate['version']}"]
                    record["selected"].append(license_record["id"])
                    record["texts"][f"selected:{license_record['id']}:{text_key}"] = text_key
        for record in dependencies.values():
            record["targets"] = sorted(set(record["targets"]))
            record["selected"] = sorted(set(record["selected"]))
            accepted = set(policy["accepted"]) | set(policy.get(record["name"], {}).get("accepted", []))
            if not record["selected"] or not set(record["selected"]) <= accepted:
                raise ValueError(f"unaccepted licence selected for {record['name']}")
        runtime_texts = {label: add_text(content) for label, content in runtime.items()}
        receipt = {"format": 1, "cargo_about": VERSION, "source_fingerprint": fingerprint,
                   "graph": graph, "targets": list(targets),
                   "rust_version": rust_version, "runtime_texts": runtime_texts,
                   "dependencies": [dependencies[key] for key in sorted(dependencies)]}
        # The human notice is also hashed in the receipt, not just its full-text inputs.
        lines = ["THIRD-PARTY NOTICES", "", f"Release graph: {graph}",
                 f"Targets: {', '.join(targets)}", f"Generated with cargo-about {VERSION}.",
                 "Full licence texts, copyright and upstream notices are in THIRD_PARTY_LICENSES/.",
                 "Upstream alternative licence files are retained, but the selected licences below govern our use.",
                 "Software is combined under GPL-3.0-only; unchanged fonts retain their own licences.", ""]
        if graph != "web":
            lines += ["FONT LICENCE FLAG: Bitstream-Vera, OFL-1.1 and Ubuntu-font-1.0 are font-only aggregation terms,",
                      "not permission to relicense the font software under GPL. See docs/RELEASING.md.", ""]
        else:
            lines += ["Browser JS in web/*.js and crates/tje-engine-wasmgen/src/browser.js is project GPL-3.0-only code.",
                      "Generated wasm-bindgen loader/glue is covered by wasm-bindgen's selected terms below.",
                      "No npm library or font file is shipped: CSS uses the browser's system fonts.", ""]
        lines += [f"RUST STANDARD LIBRARY {rust_version}: complete upstream copyright and licence report follows.",
                  "This report/reference licence set covers all upstream library targets, not a claim",
                  "that compiler/documentation or every optional component was linked into this artifact.",
                  "Source: https://github.com/rust-lang/rust (version pinned in rust-toolchain.toml)"]
        for label, text_key in sorted(runtime_texts.items()):
            lines.append(f"  {label}: THIRD_PARTY_LICENSES/{text_key}.txt")
        lines.append("")
        for key in sorted(dependencies):
            record = dependencies[key]
            lines += [key, f"Declared: {record['declared_license']}",
                      f"Selected: {', '.join(record['selected'])}",
                      f"Authors (upstream metadata): {'; '.join(author['name'] for author in record['authors'])}",
                      f"Source: {record['repository'] or 'see project source receipt'}"]
            if record["source_archive"]:
                lines.append(f"Exact upstream source archive: {record['source_archive']}")
            for label, text_key in sorted(record["texts"].items()):
                lines.append(f"  {label}: THIRD_PARTY_LICENSES/{text_key}.txt")
            lines.append("")
        outputs[f"{graph}.txt"] = ("\n".join(lines) + "\n").encode()
        receipt["notice_sha256"] = digest(outputs[f"{graph}.txt"])
        outputs[f"{graph}.json"] = (json.dumps(receipt, indent=2, sort_keys=True) + "\n").encode()
        for key, content in texts.items():
            outputs[f"texts/{key}.txt"] = content
    return outputs


def package_files(graph: str, root: Path = ROOT, *,
                  reader: Callable[[Path], bytes] = Path.read_bytes) -> dict[str, bytes]:
    """Cheap fail-closed check at packaging time; CI separately re-harvests all graphs."""
    receipt_bytes = reader(root / f"third-party/{graph}.json")
    receipt = json.loads(receipt_bytes)
    if (receipt["format"] != 1 or receipt["cargo_about"] != VERSION
            or receipt["graph"] != graph or receipt["targets"] != list(GRAPHS[graph][1])):
        raise ValueError("unsupported third-party notice receipt")
    policy = tomllib.loads(reader(root / "tools/notices.toml").decode("utf-8"))
    if receipt["source_fingerprint"] != source_fingerprint(root):
        raise ValueError("third-party notices are stale; run python tools/notices.py")
    files = {"THIRD_PARTY_NOTICES.txt": reader(root / f"third-party/{graph}.txt")}
    if digest(files["THIRD_PARTY_NOTICES.txt"]) != receipt["notice_sha256"]:
        raise ValueError("third-party notice checksum mismatch")
    for dependency in receipt["dependencies"]:
        accepted = set(policy["accepted"]) | set(policy.get(dependency["name"], {}).get("accepted", []))
        if not dependency["selected"] or not set(dependency["selected"]) <= accepted:
            raise ValueError(f"unaccepted licence for {dependency['name']}")
        for key in dependency["texts"].values():
            if not re.fullmatch(r"[0-9a-f]{64}", key):
                raise ValueError("invalid third-party licence text key")
            content = reader(root / f"third-party/texts/{key}.txt")
            if digest(content) != key:
                raise ValueError("third-party licence text checksum mismatch")
            files[f"THIRD_PARTY_LICENSES/{key}.txt"] = content
    for key in receipt["runtime_texts"].values():
        if not re.fullmatch(r"[0-9a-f]{64}", key):
            raise ValueError("invalid runtime licence text key")
        content = reader(root / f"third-party/texts/{key}.txt")
        if digest(content) != key:
            raise ValueError("runtime licence text checksum mismatch")
        files[f"THIRD_PARTY_LICENSES/{key}.txt"] = content
    files["THIRD_PARTY_DEPENDENCIES.json"] = receipt_bytes
    return files


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail on stale output or unaccepted licence")
    args = parser.parse_args()
    generated = generate(tool_path())
    destination = ROOT / "third-party"
    existing = {path.relative_to(destination).as_posix(): path.read_bytes()
                for path in destination.rglob("*") if path.is_file()}
    if args.check:
        if existing != generated:
            raise SystemExit("third-party notices are stale; run python tools/notices.py")
        print("Third-party notices current: four release graphs / five targets; all selected licences accepted")
    else:
        for name, data in generated.items():
            if existing.get(name) != data:
                path = destination / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
        for name in existing.keys() - generated.keys():
            destination.joinpath(name).unlink()
        print(f"Generated {len(generated)} third-party notice files")


if __name__ == "__main__":
    main()
