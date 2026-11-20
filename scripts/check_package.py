"""Audit both feature graphs and the default kernel's no_std boundary."""
import json
from pathlib import Path
import re
import subprocess
import tomllib

root = tomllib.loads(Path("Cargo.toml").read_text())
assert root["package"]["rust-version"] == "1.99.1"
assert root["package"]["edition"] == "2024"
assert set(root.get("dependencies", {})) == {"zconv"}
dependency = root["dependencies"]["zconv"]
assert dependency.get("default-features") is False
assert not dependency.get("features")
for key in ("dev-dependencies", "build-dependencies", "target"):
    assert not root.get(key), f"Unexpected {key}"
assert root["features"] == {"default": [], "std": [], "server": ["std"]}
assert not Path("build.rs").exists()

for feature_args, allowed in [
    (["--no-default-features"], set()),
    (["--all-features"], {"default", "std", "server"}),
]:
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--locked", *feature_args], text=True))
    packages = {package["name"]: package for package in metadata["packages"]}
    assert len(metadata["packages"]) == 2
    assert set(packages) == {"zcall", "zconv"}, set(packages)
    assert not packages["zconv"]["dependencies"]
    assert packages["zconv"]["features"] == {"default": []}
    by_id = {package["id"]: package["name"] for package in metadata["packages"]}
    for node in metadata["resolve"]["nodes"]:
        permitted = allowed if by_id[node["id"]] == "zcall" else set()
        assert not set(node["features"]) - permitted, node["features"]

for name, package in packages.items():
    directory = Path(package["manifest_path"]).parent
    assert not (directory / "build.rs").exists()
    crate_root = (directory / "src/lib.rs").read_text()
    if name == "zcall":
        assert '#![cfg_attr(not(feature = "std"), no_std)]' in crate_root
        assert '#[cfg(feature = "server")]\npub mod server;' in crate_root
    else:
        assert "#![no_std]" in crate_root
    assert "#![forbid(unsafe_code)]" in crate_root
    for path in (directory / "src").rglob("*.rs"):
        text = path.read_text()
        code = "\n".join(line.split("//", 1)[0] for line in text.splitlines())
        assert not re.search(r"\bunsafe\s*(?:\{|fn|impl|trait)", code), path
        optional_server = name == "zcall" and path.is_relative_to(directory / "src/server")
        if not optional_server:
            assert not re.search(r"\b(?:alloc|std)::|extern\s+crate\s+(?:alloc|std)", code), path

print("PASS: both graphs are exactly zcall -> zconv; default kernel is core-only; "
      "std is confined to the opt-in server module; all source forbids unsafe")
