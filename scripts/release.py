"""Version and registry gates. Input comes from environment, never shell interpolation."""
from contextlib import chdir
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tomllib
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from urllib.parse import parse_qsl, urlsplit

SEMVER = re.compile(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\Z')
MAX_COMPONENT = 2**32 - 1
CRATES_IO_SOURCE = 'registry+https://github.com/rust-lang/crates.io-index'
SIDECAR_LOCKS = (Path('comparison/Cargo.lock'), Path('fuzz/Cargo.lock'))
RELEASE_FILES = (Path('Cargo.toml'), Path('Cargo.lock'), *SIDECAR_LOCKS,
                 Path('README.md'), Path('CHANGELOG.md'))
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


def request():
    """Require an explicit, matching main-branch release request for push events."""
    if os.environ.get('GITHUB_REF') != 'refs/heads/main':
        raise ValueError('release requests must originate from main')
    data = json.loads(Path('.github/release-request.json').read_text())
    if not isinstance(data, dict) or set(data) != {'crate', 'version', 'zconv', 'publish'}:
        raise ValueError('release request must contain only the required keys')
    package = tomllib.loads(Path('Cargo.toml').read_text())
    expected = {
        'crate': package['package']['name'],
        'version': package['package']['version'],
        'zconv': '=' + dependency_version(package),
        'publish': True,
    }
    if data != expected or data['publish'] is not True:
        raise ValueError('release request does not match the exact crate and dependency')
    version_tuple(data['version'])
    print(f"Validated explicit release request for {data['crate']} {data['version']}")


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


def dependency_version(package):
    """Bind Git development input to its committed lock, never a registry namesake."""
    spec = package['dependencies']['zconv']
    if not isinstance(spec, dict) or spec.get('package', 'zconv') != 'zconv':
        raise ValueError('expected the zconv package')
    if spec.get('default-features', True) is not False or spec.get('features'):
        raise ValueError('zconv must disable default and additional features')
    if 'path' in spec:
        raise ValueError('local dependency paths are forbidden for publication')
    if 'git' not in spec:
        if any(key in spec for key in ('rev', 'branch', 'tag')):
            raise ValueError('Git selectors require a Git dependency')
        exact = spec.get('version', '')
        if not isinstance(exact, str) or not exact.startswith('='):
            raise ValueError('zconv must have an exact registry version')
        version_tuple(exact[1:])
        return exact[1:]

    git = spec['git']
    if git not in ('https://github.com/LIghtJUNction/zconv.git',
                   'https://github.com/LIghtJUNction/zconv'):
        raise ValueError('unexpected zconv Git repository')
    committed = subprocess.check_output(['git', 'show', 'HEAD:Cargo.lock'], text=True)
    _, locked = unique_lock_package(committed, 'zconv', 'committed Cargo.lock')
    _, working = unique_lock_package(Path('Cargo.lock').read_text(), 'zconv', 'Cargo.lock')
    if working != locked:
        raise ValueError('working zconv lock differs from committed Cargo.lock')
    source = locked.get('source', '')
    if not isinstance(source, str) or not source.startswith('git+'):
        raise ValueError('committed zconv package must use the declared Git source')
    parsed = urlsplit(source[4:])
    selectors = [(key, spec[key]) for key in ('rev', 'branch', 'tag') if key in spec]
    if (len(selectors) > 1 or
            parsed._replace(query='', fragment='').geturl() != git or
            parse_qsl(parsed.query, keep_blank_values=True, strict_parsing=True) != selectors or
            not re.fullmatch(r'[0-9a-f]{40}', parsed.fragment)):
        raise ValueError('committed zconv Git identity does not match its declaration')
    if 'rev' in spec and (not isinstance(spec['rev'], str) or
                          not re.fullmatch(r'[0-9a-f]{7,40}', spec['rev']) or
                          not parsed.fragment.startswith(spec['rev'])):
        raise ValueError('committed zconv revision does not match its declared commit')
    version = locked.get('version', '')
    if not isinstance(version, str):
        raise ValueError('committed zconv package version must be a string')
    version_tuple(version)
    if 'version' in spec and spec['version'] != '=' + version:
        raise ValueError('declared zconv version differs from the committed Git package')
    if locked.get('dependencies'):
        raise ValueError('committed zconv unexpectedly has transitive dependencies')
    return version


def create_release_stage():
    """Use a detached copy for registry projection; retain prepared Git development files."""
    source = Path.cwd().resolve()
    root = Path(subprocess.check_output(
        ['git', 'rev-parse', '--show-toplevel'], text=True).strip()).resolve()
    if source != root:
        raise ValueError('release staging must start at the repository root')
    stage = source / 'target/release-stage'
    if stage.exists() or stage.is_symlink() or not stage.resolve().is_relative_to(source):
        raise ValueError('release stage must be a new directory inside this checkout')
    subprocess.run(['git', 'worktree', 'add', '--detach', str(stage), 'HEAD'], check=True)
    # prepare() may have bumped only the local version and its documentation.
    for path in RELEASE_FILES:
        if path.exists():
            shutil.copy2(path, stage / path)
    return stage


def registry_manifest(data, version):
    """Project only the zconv declaration; retain its other TOML options."""
    spec = tomllib.loads(data)['dependencies']['zconv']
    projected = {key: value for key, value in spec.items()
                 if key not in ('version', 'git', 'rev', 'branch', 'tag')}
    projected = {'version': '=' + version, **projected}
    inline = ', '.join(f'{key} = {json.dumps(value)}' for key, value in projected.items())
    data, count = re.subn(r'(?m)^zconv\s*=\s*\{[^\n]*\}[ \t]*$',
                         lambda _: 'zconv = { ' + inline + ' }', data)
    if count != 1 or tomllib.loads(data)['dependencies']['zconv'] != projected:
        raise ValueError('expected one single-line zconv dependency')
    return data


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
    current = package['version']
    chosen = select_version(current, os.getenv('BUMP', 'none'),
                            os.getenv('VERSION', ''), os.getenv('RELEASE_TAG', ''))
    originals = {path: path.read_bytes() if path.exists() else None for path in RELEASE_FILES}
    updates = {}
    if chosen != current:
        for path in SIDECAR_LOCKS:
            if path.exists():
                updates[path] = update_path_lock(path.read_text(), package['name'], current, chosen)
        data, count = re.subn(r'(?m)^version = "' + re.escape(current) + r'"$',
                             f'version = "{chosen}"', Path('Cargo.toml').read_text(), count=1)
        if count != 1:
            raise ValueError('expected one package version field')
        updates[Path('Cargo.toml')] = data
    readme = Path('README.md')
    if readme.exists():
        updates[readme] = re.sub(
            r'(zcall\s*=\s*\{\s*version\s*=\s*")' + re.escape(current) + '"',
            lambda match: match[1] + chosen + '"', readme.read_text())
    changelog = Path('CHANGELOG.md')
    if changelog.exists():
        data = changelog.read_text()
        if re.search(r'(?m)^## Unreleased[ \t]*$', data):
            data = re.sub(r'(?m)^## Unreleased[ \t]*$', f'## {chosen}', data, count=1)
        elif chosen != current and not re.search(r'(?m)^## ' + re.escape(chosen) + '$', data):
            data = data.replace('# Changelog\n', f'# Changelog\n\n## {chosen}\n', 1)
        updates[changelog] = data
    try:
        if chosen != current:
            Path('Cargo.toml').write_text(updates.pop(Path('Cargo.toml')))
            # The exact dependency stays pinned; Cargo updates the local version.
            subprocess.run(['cargo', '+1.99.1', 'update', '--workspace', '--offline'], check=True)
        for path, data in updates.items():
            path.write_text(data)
    except BaseException:
        for path, previous in originals.items():
            if previous is None:
                path.unlink(missing_ok=True)
            else:
                path.write_bytes(previous)
        raise
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
            raise ValueError('release source is not part of main')
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
             'comparison/Cargo.lock', 'fuzz/Cargo.lock', 'README.md', 'CHANGELOG.md'], check=False)
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
    version = dependency_version(package)
    if release['num'] != version or release.get('yanked', False):
        raise ValueError('required zconv version is absent or yanked')
    return version


def dependency():
    package = tomllib.loads(Path('Cargo.toml').read_text())
    version = dependency_version(package)
    # A 404 or network failure is a hard release gate; do not fall back to Git.
    published = get_json(f'https://crates.io/api/v1/crates/zconv/{version}')['version']
    validate_dependency(package, published)
    stage = create_release_stage()
    with chdir(stage):
        path = Path('Cargo.toml')
        path.write_text(registry_manifest(path.read_text(), version))
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
    print(f'{stage}: crates.io zconv {version} resolved without Git/path or transitive dependencies')
    return stage


if __name__ == '__main__':
    commands = {'prepare': prepare, 'registry': registry, 'git-preflight': git_preflight,
                'dependency': dependency, 'request': request}
    if len(sys.argv) != 2 or sys.argv[1] not in commands:
        raise SystemExit('usage: release.py prepare|registry|git-preflight|dependency|request')
    commands[sys.argv[1]]()
