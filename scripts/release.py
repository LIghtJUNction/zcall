"""Version and registry gates. Input comes from environment, never shell interpolation."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib
from urllib.error import HTTPError
from urllib.request import Request, urlopen

SEMVER = re.compile(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\Z')
MAX_COMPONENT = 2**32 - 1
CRATES_IO_SOURCE = 'registry+https://github.com/rust-lang/crates.io-index'
SIDECAR_LOCKS = (Path('comparison/Cargo.lock'), Path('fuzz/Cargo.lock'))
LOCK_PACKAGES = re.compile(r'(?ms)^\[\[package\]\]\n.*?(?=^\[|\Z)')


def version_tuple(value):
    match = SEMVER.fullmatch(value)
    if not match:
        raise ValueError('version must be stable SemVer: MAJOR.MINOR.PATCH')
    result = tuple(map(int, match.groups()))
    if any(v > MAX_COMPONENT for v in result):
        raise ValueError('SemVer component exceeds supported range')
    return result


def select_version(current, bump, explicit='', tag=''):
    old = version_tuple(current)
    if tag:
        if tag != 'v' + current:
            raise ValueError('release tag does not match Cargo.toml')
        if explicit and explicit != current:
            raise ValueError('explicit version conflicts with tag')
        return current
    if explicit:
        if bump != 'none':
            raise ValueError('explicit version requires bump=none')
        if version_tuple(explicit) < old:
            raise ValueError('version must not decrease')
        return explicit
    if bump not in ('none', 'patch', 'minor', 'major'):
        raise ValueError('unknown bump mode')
    major, minor, patch = old
    if bump == 'major':
        major, minor, patch = major + 1, 0, 0
    elif bump == 'minor':
        minor, patch = minor + 1, 0
    elif bump == 'patch':
        patch += 1
    result = f'{major}.{minor}.{patch}'
    version_tuple(result)
    return result


def manifest():
    return tomllib.loads(Path('Cargo.toml').read_text())['package']


def update_path_lock(data, name, current, chosen):
    """Update only the local crate stanza; retain every registry pin and checksum."""
    count = 0

    def replace(match):
        nonlocal count
        block = match.group()
        package = tomllib.loads(block)['package'][0]
        if package['name'] != name or 'source' in package:
            return block
        if package['version'] != current:
            raise ValueError(f'{name} path dependency lock is out of date')
        count += 1
        updated, replaced = re.subn(
            r'(?m)^version = "' + re.escape(current) + r'"$',
            f'version = "{chosen}"', block, count=1)
        if replaced != 1:
            raise ValueError('expected one path dependency version field')
        return updated

    result = re.sub(r'(?ms)^\[\[package\]\]\n.*?(?=^\[\[package\]\]|\Z)', replace, data)
    if count != 1:
        raise ValueError(f'expected one {name} path dependency in lockfile')
    return result


def unique_lock_package(data, name, location):
    """Locate a package without serializing unrelated lockfile contents."""
    tomllib.loads(data)
    matches = []
    for match in LOCK_PACKAGES.finditer(data):
        package = tomllib.loads(match.group())['package'][0]
        if package['name'] == name:
            matches.append((match, package))
    if len(matches) != 1:
        raise ValueError(f'expected one {name} package in {location}')
    return matches[0]


def synchronize_dependency_locks(version):
    """Copy only verified zconv resolution into existing independent projects."""
    main, package = unique_lock_package(Path('Cargo.lock').read_text(), 'zconv', 'Cargo.lock')
    if package['version'] != version or package.get('source') != CRATES_IO_SOURCE:
        raise ValueError('main zconv lock does not match the verified registry dependency')
    checksum = package.get('checksum', '')
    if not isinstance(checksum, str) or not re.fullmatch(r'[0-9a-f]{64}', checksum):
        raise ValueError('verified registry dependency must have a Cargo checksum')
    if package.get('dependencies'):
        raise ValueError('verified zconv lock unexpectedly has transitive dependencies')
    replacement = main.group().rstrip('\n')
    updates = {}
    for path in SIDECAR_LOCKS:
        if not path.exists():
            continue
        data = path.read_text()
        original, old_package = unique_lock_package(data, 'zconv', path)
        if old_package['version'] != version:
            raise ValueError(f'zconv version in {path} is stale; resolve that project first')
        # Preserve the independent project's lock version, pins and separators.
        trailing = original.group()[len(original.group().rstrip('\n')):]
        updates[path] = data[:original.start()] + replacement + trailing + data[original.end():]
    # Validate every existing lock before changing any of them.
    for path, data in updates.items():
        path.write_text(data)


def prepare():
    package = manifest()
    chosen = select_version(package['version'], os.getenv('BUMP', 'none'),
                            os.getenv('VERSION', ''), os.getenv('RELEASE_TAG', ''))
    if chosen != package['version']:
        lock_updates = {}
        for path in SIDECAR_LOCKS:
            if path.exists():
                lock_updates[path] = update_path_lock(
                    path.read_text(), package['name'], package['version'], chosen)
        path = Path('Cargo.toml')
        data, count = re.subn(r'(?m)^version = "' + re.escape(package['version']) + r'"$',
                             f'version = "{chosen}"', path.read_text(), count=1)
        if count != 1:
            raise ValueError('expected one package version field')
        path.write_text(data)
        # Generate with the declared MSRV; Cargo 1.99.1 supports the v4 lockfile.
        subprocess.run(['cargo', '+1.99.1', 'generate-lockfile', '--offline'], check=True)
        for path, data in lock_updates.items():
            path.write_text(data)
    output = os.getenv('GITHUB_OUTPUT')
    if output:
        with open(output, 'a', encoding='utf-8') as file:
            file.write(f'version={chosen}\n')
    print(f'zcall {chosen}')


def git_preflight():
    """Reject stale manual releases and tags that would label different source."""
    version = manifest()['version']
    version_tuple(version)
    expected = os.environ['GITHUB_SHA']
    event = os.environ['GITHUB_EVENT_NAME']
    if event not in ('workflow_dispatch', 'push'):
        raise ValueError('unexpected release event')
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
    if head != expected:
        raise ValueError('checkout differs from the verified workflow commit')
    subprocess.run(['git', 'fetch', '--no-recurse-submodules', '--tags', 'origin', 'main'], check=True)
    if event == 'workflow_dispatch':
        latest = subprocess.check_output(['git', 'rev-parse', 'FETCH_HEAD'], text=True).strip()
        if latest != expected:
            raise ValueError('main advanced after verification; rerun the release on its new head')
    else:
        reachability = subprocess.run(['git', 'merge-base', '--is-ancestor', expected, 'FETCH_HEAD'], check=False)
        if reachability.returncode != 0:
            raise ValueError('release tag source is not part of main')
    tag = subprocess.run(
        ['git', 'rev-parse', '--verify', '--quiet', f'refs/tags/v{version}^{{commit}}'],
        text=True, stdout=subprocess.PIPE, check=False)
    if tag.returncode not in (0, 1):
        raise ValueError('could not inspect the release tag')
    if tag.returncode == 0:
        if tag.stdout.strip() != head:
            raise ValueError('release tag points to a different commit')
        changes = subprocess.run(
            ['git', 'diff', '--quiet', 'HEAD', '--', 'Cargo.toml', 'Cargo.lock',
             'comparison/Cargo.lock', 'fuzz/Cargo.lock'], check=False)
        if changes.returncode != 0:
            raise ValueError('release tag does not include the prepared version changes')
    print('Verified checkout, default branch and release tag are consistent')


def get_json(url):
    request = Request(url, headers={'User-Agent': 'zcall-release (https://github.com/LIghtJUNction/zcall)'})
    with urlopen(request, timeout=30) as response:
        return json.load(response)


def registry():
    package = manifest()
    name, version = package['name'], package['version']
    if name != 'zcall':
        raise ValueError('crate naming must be changed coherently before release')
    api = f'https://crates.io/api/v1/crates/{name}'
    try:
        data = get_json(api)
    except HTTPError as error:
        if error.code != 404:
            raise
        print('Registry name is currently unoccupied; first publication may proceed')
        return
    repository = (data['crate'].get('repository') or '').rstrip('/').removesuffix('.git')
    if repository.lower() != 'https://github.com/lightjunction/zcall':
        raise ValueError('registry name is occupied by a different repository; stop and rename coherently')
    if any(item['num'] == version for item in data['versions']):
        raise ValueError(f'{name} {version} already exists; select a new version')
    owners = get_json(api + '/owners')['users']
    if not any(user.get('login', '').lower() == 'lightjunction' for user in owners):
        raise ValueError('expected registry owner is absent')
    print('Registry owner, repository and unpublished version verified')


def validate_dependency(package, release):
    """A Git checkout is a development input, never the published dependency."""
    spec = package['dependencies']['zconv']
    version = spec['version']
    if not version.startswith('='):
        raise ValueError('zconv must have an exact registry version')
    version_tuple(version[1:])
    if release['num'] != version[1:] or release.get('yanked', False):
        raise ValueError('required zconv version is absent or yanked')
    if spec.get('default-features', True) or spec.get('features'):
        raise ValueError('zconv must disable default and additional features')
    if spec.get('path'):
        raise ValueError('local dependency paths are forbidden for publication')
    return version[1:]


def dependency():
    path = Path('Cargo.toml')
    package = tomllib.loads(path.read_text())
    spec = package['dependencies']['zconv']
    version = spec['version'].removeprefix('=')
    version_tuple(version)
    # A 404 or network failure is a hard release gate; do not fall back to Git.
    release = get_json(f'https://crates.io/api/v1/crates/zconv/{version}')['version']
    validate_dependency(package, release)
    data = path.read_text()
    if 'git' in spec or 'rev' in spec:
        lines = data.splitlines(keepends=True)
        matched = 0
        for i, line in enumerate(lines):
            if re.match(r'^zconv\s*=\s*\{', line):
                line = re.sub(r',\s*(?:git|rev)\s*=\s*"[^"\n]+"', '', line)
                lines[i] = line
                matched += 1
        if matched != 1:
            raise ValueError('expected one single-line zconv dependency')
        path.write_text(''.join(lines))
    subprocess.run(['cargo', '+1.99.1', 'generate-lockfile'], check=True)
    resolved = json.loads(subprocess.check_output(
        ['cargo', '+1.99.1', 'metadata', '--locked', '--format-version', '1'], text=True))
    dependencies = [p for p in resolved['packages'] if p['name'] == 'zconv']
    if len(dependencies) != 1 or dependencies[0]['version'] != version:
        raise ValueError('unexpected resolved zconv version')
    if dependencies[0]['source'] != CRATES_IO_SOURCE:
        raise ValueError('zconv was not resolved from crates.io')
    if dependencies[0]['dependencies']:
        raise ValueError('zconv unexpectedly has transitive dependencies')
    synchronize_dependency_locks(version)
    print(f'crates.io dependency zconv {version} resolved without Git/path or transitive dependencies')


if __name__ == '__main__':
    commands = {'prepare': prepare, 'registry': registry, 'git-preflight': git_preflight, 'dependency': dependency}
    if len(sys.argv) != 2 or sys.argv[1] not in commands:
        raise SystemExit('usage: release.py prepare|registry|git-preflight|dependency')
    commands[sys.argv[1]]()
