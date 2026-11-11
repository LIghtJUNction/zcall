"""Audit the actual resolved graph, including features and target-specific deps."""
import json
from pathlib import Path
import re
import subprocess
import tomllib

root = tomllib.loads(Path('Cargo.toml').read_text())
assert root['package']['rust-version'] == '1.99.1'
assert root['package']['edition'] == '2024'
assert set(root.get('dependencies', {})) == {'zconv'}
dependency = root['dependencies']['zconv']
assert dependency.get('default-features') is False
assert not dependency.get('features')
for key in ('dev-dependencies', 'build-dependencies', 'target'):
    assert not root.get(key), f'Unexpected {key}'
assert root['features'] == {'default': []}
assert not Path('build.rs').exists()

metadata = json.loads(subprocess.check_output(
    ['cargo', 'metadata', '--format-version', '1', '--locked'], text=True))
packages = {p['name']: p for p in metadata['packages']}
assert set(packages) == {'zcall', 'zconv'}, set(packages)
assert not packages['zconv']['dependencies']
assert packages['zconv']['features'] == {'default': []}
for node in metadata['resolve']['nodes']:
    assert not set(node['features']) - {'default'}, node['features']

for package in packages.values():
    directory = Path(package['manifest_path']).parent
    assert not (directory / 'build.rs').exists()
    crate_root = (directory / 'src/lib.rs').read_text()
    assert '#![no_std]' in crate_root
    assert '#![forbid(unsafe_code)]' in crate_root
    for path in (directory / 'src').rglob('*.rs'):
        text = path.read_text()
        # Ignore prose comments. Tests live outside both production source trees.
        code = '\n'.join(line.split('//', 1)[0] for line in text.splitlines())
        assert not re.search(r'\b(?:alloc|std)::|extern\s+crate\s+(?:alloc|std)', code), path
        assert not re.search(r'\bunsafe\s*(?:\{|fn|impl|trait)', code), path
print('PASS: exactly zcall -> zconv; core-only source; no unsafe, alloc, std or build scripts')
