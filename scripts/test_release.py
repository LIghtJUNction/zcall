from contextlib import chdir, redirect_stdout
from io import StringIO
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest
from unittest.mock import patch
from urllib.error import HTTPError

import release
from release import select_version, update_path_lock


REGISTRY_ZCONV = '''[[package]]
name = "zconv"
version = "0.2.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "''' + 'a' * 64 + '''"
'''
GIT_ZCONV = '''[[package]]
name = "zconv"
version = "0.2.0"
source = "git+https://github.com/LIghtJUNction/zconv?rev=abc#abc"
'''
INDEPENDENT_LOCK = '''# Keep independent project pins and lockfile format.
version = 4

[[package]]
name = "oracle"
version = "1.2.3"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "unchanged-before"

[[package]]
name = "zcall"
version = "0.1.0"
dependencies = ["zconv"]

''' + GIT_ZCONV + '''
[[package]]
name = "zz-last"
version = "9.8.7"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "unchanged-after"
'''


class DependencyTests(unittest.TestCase):
    def test_exact_published_no_feature_dependency(self):
        package={'dependencies': {'zconv': {'version':'=0.2.0','default-features':False}}}
        self.assertEqual(release.validate_dependency(package,{'num':'0.2.0','yanked':False}),'0.2.0')

    def test_unpublished_yanked_nonexact_and_features_are_rejected(self):
        for spec, published in [
            ({'version':'0.2','default-features':False},{'num':'0.2.0'}),
            ({'version':'=0.2.0','default-features':False},{'num':'0.2.1'}),
            ({'version':'=0.2.0','default-features':False},{'num':'0.2.0','yanked':True}),
            ({'version':'=0.2.0'},{'num':'0.2.0'}),
            ({'version':'=0.2.0','default-features':False,'features':['std']},{'num':'0.2.0'}),
            ({'version':'=0.2.0','default-features':False,'path':'../zconv'},{'num':'0.2.0'}),
        ]:
            with self.assertRaises(ValueError):
                release.validate_dependency({'dependencies':{'zconv':spec}},published)


class DependencyLockTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.enterContext(chdir(directory.name))
        Path('Cargo.toml').write_text('''[package]
name = "zcall"
version = "0.1.0"

[dependencies]
zconv = { version = "=0.2.0", default-features = false, git = "https://github.com/LIghtJUNction/zconv", rev = "abc" }
''')
        Path('Cargo.lock').write_text('version = 3\n\n' + REGISTRY_ZCONV)
        for project in ('comparison', 'fuzz'):
            Path(project).mkdir()
            Path(project, 'Cargo.lock').write_text(INDEPENDENT_LOCK)

    def test_registry_sync_preserves_every_other_byte_and_lock_version(self):
        Path('comparison/Cargo.lock').write_text(INDEPENDENT_LOCK.replace('version = 4', 'version = 3', 1))
        before = {path: path.read_text() for path in release.SIDECAR_LOCKS}
        release.synchronize_dependency_locks('0.2.0')
        for path, original in before.items():
            self.assertEqual(path.read_text(), original.replace(GIT_ZCONV, REGISTRY_ZCONV))
            self.assertEqual(tomllib.loads(path.read_text())['version'],
                             tomllib.loads(original)['version'])
        first = {path: path.read_text() for path in before}
        release.synchronize_dependency_locks('0.2.0')
        self.assertEqual(first, {path: path.read_text() for path in before})

    def test_absent_independent_project_is_not_created(self):
        Path('comparison/Cargo.lock').unlink()
        Path('comparison').rmdir()
        release.synchronize_dependency_locks('0.2.0')
        self.assertFalse(Path('comparison').exists())
        self.assertEqual(Path('fuzz/Cargo.lock').read_text(),
                         INDEPENDENT_LOCK.replace(GIT_ZCONV, REGISTRY_ZCONV))

    def test_missing_duplicate_or_stale_sidecar_does_not_partially_update_others(self):
        invalid = (
            INDEPENDENT_LOCK.replace(GIT_ZCONV, ''),
            INDEPENDENT_LOCK + '\n' + GIT_ZCONV,
            INDEPENDENT_LOCK.replace('name = "zconv"\nversion = "0.2.0"',
                                     'name = "zconv"\nversion = "0.1.0"'),
        )
        for lock in invalid:
            with self.subTest(lock=lock):
                Path('fuzz/Cargo.lock').write_text(lock)
                with self.assertRaises(ValueError):
                    release.synchronize_dependency_locks('0.2.0')
                self.assertEqual(Path('comparison/Cargo.lock').read_text(), INDEPENDENT_LOCK)
                self.assertEqual(Path('fuzz/Cargo.lock').read_text(), lock)

    def test_only_verified_registry_stanza_can_be_copied(self):
        invalid = (
            GIT_ZCONV,
            REGISTRY_ZCONV.replace('version = "0.2.0"', 'version = "0.2.1"'),
            REGISTRY_ZCONV.replace('a' * 64, 'not-a-checksum'),
            REGISTRY_ZCONV + 'dependencies = ["unexpected"]\n',
            REGISTRY_ZCONV + '\n' + REGISTRY_ZCONV,
        )
        for stanza in invalid:
            with self.subTest(stanza=stanza):
                Path('Cargo.lock').write_text('version = 3\n\n' + stanza)
                with self.assertRaises(ValueError):
                    release.synchronize_dependency_locks('0.2.0')
                for path in release.SIDECAR_LOCKS:
                    self.assertEqual(path.read_text(), INDEPENDENT_LOCK)

    def resolve_dependency(self, package):
        def generate_lock(command, *, check):
            self.assertEqual(command, ['cargo', '+1.99.1', 'generate-lockfile'])
            self.assertTrue(check)
            Path('Cargo.lock').write_text('version = 3\n\n' + REGISTRY_ZCONV)

        with patch.object(release, 'get_json', return_value={'version': {'num': '0.2.0', 'yanked': False}}), \
                patch.object(release.subprocess, 'run', side_effect=generate_lock), \
                patch.object(release.subprocess, 'check_output', return_value=json.dumps({'packages': [package]})), \
                redirect_stdout(StringIO()):
            release.dependency()

    def test_dependency_gate_synchronizes_git_source_without_a_version_bump(self):
        Path('Cargo.lock').write_text('version = 3\n\n' + GIT_ZCONV)
        self.resolve_dependency({'name': 'zconv', 'version': '0.2.0',
                                 'source': release.CRATES_IO_SOURCE, 'dependencies': []})
        manifest = tomllib.loads(Path('Cargo.toml').read_text())
        self.assertEqual(manifest['package']['version'], '0.1.0')
        self.assertNotIn('git', manifest['dependencies']['zconv'])
        self.assertNotIn('rev', manifest['dependencies']['zconv'])
        for path in release.SIDECAR_LOCKS:
            self.assertEqual(path.read_text(), INDEPENDENT_LOCK.replace(GIT_ZCONV, REGISTRY_ZCONV))

    def test_failed_registry_resolution_does_not_touch_independent_locks(self):
        resolved = {'name': 'zconv', 'version': '0.2.0',
                    'source': release.CRATES_IO_SOURCE, 'dependencies': []}
        for change in ({'source': 'git+unverified'}, {'version': '0.2.1'}, {'dependencies': ['unexpected']}):
            with self.subTest(change=change):
                with self.assertRaises(ValueError):
                    self.resolve_dependency({**resolved, **change})
                for path in release.SIDECAR_LOCKS:
                    self.assertEqual(path.read_text(), INDEPENDENT_LOCK)


class ReleaseTests(unittest.TestCase):
    def test_bumps(self):
        for mode, expected in [('none', '1.2.3'), ('patch', '1.2.4'), ('minor', '1.3.0'), ('major', '2.0.0')]:
            self.assertEqual(select_version('1.2.3', mode), expected)

    def test_injection_and_invalid_semver(self):
        for value in ['01.2.3', '1.2', 'v1.2.3', '1.2.3\n', '1.2.3;echo bad', '$(id)', '1.2.3-rc.1']:
            with self.assertRaises(ValueError):
                select_version('1.2.3', 'none', value)

    def test_conflicting_inputs_and_downgrade(self):
        for args in [('1.2.3', 'patch', '1.2.4'), ('1.2.3', 'none', '1.2.2'), ('1.2.3', 'none', '', 'v1.2.4')]:
            with self.assertRaises(ValueError):
                select_version(*args)

    def test_tag_is_exact(self):
        self.assertEqual(select_version('0.1.0', 'none', '', 'v0.1.0'), '0.1.0')


class PrepareTests(unittest.TestCase):
    LOCK = '''# Preserve the lockfile version and registry resolution.
version = 4

[[package]]
name = "oracle"
version = "1.2.3"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "unchanged"

[[package]]
name = "zcall"
version = "1.2.3"
'''

    def test_path_lock_update_retains_registry_resolution(self):
        updated = update_path_lock(self.LOCK, 'zcall', '1.2.3', '1.2.4')
        before, after = tomllib.loads(self.LOCK), tomllib.loads(updated)
        self.assertEqual(after['version'], 4)
        self.assertEqual(before['package'][0], after['package'][0])
        self.assertEqual(after['package'][1], {'name': 'zcall', 'version': '1.2.4'})

    def test_path_lock_rejects_missing_stale_and_duplicate_stanzas(self):
        for data in (self.LOCK.replace('name = "zcall"', 'name = "different"'),
                     self.LOCK.replace('name = "zcall"\nversion = "1.2.3"',
                                       'name = "zcall"\nversion = "1.0.0"'),
                     self.LOCK + '\n[[package]]\nname = "zcall"\nversion = "1.2.3"\n'):
            with self.assertRaises(ValueError):
                update_path_lock(data, 'zcall', '1.2.3', '1.2.4')

    def test_prepare_bumps_every_local_lock_then_is_idempotent(self):
        with tempfile.TemporaryDirectory() as directory, chdir(directory):
            Path('Cargo.toml').write_text('[package]\nname = "zcall"\nversion = "1.2.3"\n')
            for project in ('comparison', 'fuzz'):
                Path(project).mkdir()
                Path(project, 'Cargo.lock').write_text(self.LOCK)
            environment = {'BUMP': 'patch', 'VERSION': '', 'RELEASE_TAG': '',
                           'GITHUB_OUTPUT': str(Path(directory, 'output'))}
            with patch.dict(os.environ, environment), patch.object(release.subprocess, 'run') as cargo:
                with redirect_stdout(StringIO()):
                    release.prepare()
                cargo.assert_called_once_with(
                    ['cargo', '+1.99.1', 'generate-lockfile', '--offline'], check=True)
            self.assertEqual(release.manifest()['version'], '1.2.4')
            expected = update_path_lock(self.LOCK, 'zcall', '1.2.3', '1.2.4')
            for project in ('comparison', 'fuzz'):
                self.assertEqual(Path(project, 'Cargo.lock').read_text(), expected)
            self.assertEqual(Path('output').read_text(), 'version=1.2.4\n')
            with patch.dict(os.environ, {**environment, 'BUMP': 'none'}), \
                    patch.object(release.subprocess, 'run') as cargo, redirect_stdout(StringIO()):
                release.prepare()
                cargo.assert_not_called()
            self.assertEqual(Path('comparison/Cargo.lock').read_text(), expected)


class GitPreflightTests(unittest.TestCase):
    def git(self, *args):
        return subprocess.check_output(['git', *args], text=True, stderr=subprocess.PIPE).strip()

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.enterContext(chdir(self.directory.name))
        self.git('init', '--bare', 'remote.git')
        Path('repo').mkdir()
        self.enterContext(chdir('repo'))
        self.git('init', '-b', 'main')
        self.git('config', 'user.name', 'Release test')
        self.git('config', 'user.email', 'test@example.invalid')
        Path('Cargo.toml').write_text('[package]\nname = "zcall"\nversion = "1.2.3"\n')
        self.git('add', 'Cargo.toml')
        self.git('commit', '-m', 'initial')
        self.git('remote', 'add', 'origin', '../remote.git')
        self.git('push', 'origin', 'main')
        self.head = self.git('rev-parse', 'HEAD')
        self.enterContext(patch.dict(os.environ, {
            'GITHUB_SHA': self.head, 'GITHUB_EVENT_NAME': 'workflow_dispatch'}))
        self.enterContext(redirect_stdout(StringIO()))

    def test_current_commit_and_matching_tag_are_valid(self):
        release.git_preflight()
        self.git('tag', '-a', 'v1.2.3', '-m', 'release')
        self.git('push', 'origin', 'v1.2.3')
        with patch.dict(os.environ, {'GITHUB_EVENT_NAME': 'push'}):
            release.git_preflight()

    def test_existing_tag_must_include_the_prepared_version(self):
        self.git('tag', 'v1.2.4')
        Path('Cargo.toml').write_text('[package]\nname = "zcall"\nversion = "1.2.4"\n')
        with self.assertRaisesRegex(ValueError, 'prepared version changes'):
            release.git_preflight()

    def test_existing_tag_cannot_point_to_different_source(self):
        self.git('tag', 'v1.2.3')
        Path('change').write_text('new source')
        self.git('add', 'change')
        self.git('commit', '-m', 'change')
        self.git('push', 'origin', 'main')
        with patch.dict(os.environ, {'GITHUB_SHA': self.git('rev-parse', 'HEAD')}):
            with self.assertRaisesRegex(ValueError, 'different commit'):
                release.git_preflight()

    def test_manual_release_rejects_a_branch_advanced_during_verification(self):
        Path('change').write_text('new source')
        self.git('add', 'change')
        self.git('commit', '-m', 'change')
        self.git('push', 'origin', 'main')
        self.git('checkout', '--detach', self.head)
        with self.assertRaisesRegex(ValueError, 'main advanced'):
            release.git_preflight()

    def test_tag_on_unmerged_source_is_rejected(self):
        self.git('checkout', '-b', 'unmerged')
        Path('unmerged-source').write_text('not part of main')
        self.git('add', 'unmerged-source')
        self.git('commit', '-m', 'unmerged source')
        self.git('tag', 'v1.2.3')
        with patch.dict(os.environ, {'GITHUB_SHA':self.git('rev-parse','HEAD'), 'GITHUB_EVENT_NAME':'push'}):
            with self.assertRaisesRegex(ValueError, 'not part of main'):
                release.git_preflight()


class RegistryTests(unittest.TestCase):
    def setUp(self):
        self.enterContext(patch.object(release, 'manifest', return_value={'name': 'zcall', 'version': '1.2.3'}))
        self.enterContext(redirect_stdout(StringIO()))

    def test_unoccupied_name_and_confirmed_owner(self):
        with patch.object(release, 'get_json', side_effect=HTTPError('url', 404, 'missing', {}, None)):
            release.registry()
        data = {'crate': {'repository': 'https://github.com/LIghtJUNction/zcall.git'}, 'versions': []}
        with patch.object(release, 'get_json', side_effect=[data, {'users': [{'login': 'LIghtJUNction'}]}]):
            release.registry()

    def test_registry_failures_do_not_look_like_an_unoccupied_name(self):
        with patch.object(release, 'get_json', side_effect=HTTPError('url', 503, 'unavailable', {}, None)):
            with self.assertRaises(HTTPError):
                release.registry()
        for repository in (None, 'https://github.com/other/zcall'):
            with patch.object(release, 'get_json', return_value={'crate': {'repository': repository}}):
                with self.assertRaisesRegex(ValueError, 'different repository'):
                    release.registry()

    def test_published_version_and_missing_owner_are_rejected(self):
        data = {'crate': {'repository': 'https://github.com/LIghtJUNction/zcall'},
                'versions': [{'num': '1.2.3'}]}
        with patch.object(release, 'get_json', return_value=data):
            with self.assertRaisesRegex(ValueError, 'already exists'):
                release.registry()
        data['versions'] = []
        with patch.object(release, 'get_json', side_effect=[data, {'users': []}]):
            with self.assertRaisesRegex(ValueError, 'owner is absent'):
                release.registry()


class PublishedSourceTests(unittest.TestCase):
    def run_record_step(self, projects, change_root):
        workflow = Path(__file__).resolve().parents[1] / '.github/workflows/release.yml'
        block = workflow.read_text().split('      - name: Record the published source\n', 1)[1]
        block = block.split('        run: |\n', 1)[1]
        lines = []
        for line in block.splitlines():
            if line and not line.startswith('          '):
                break
            lines.append(line[10:])
        script = '\n'.join(lines)

        def git(*args):
            return subprocess.check_output(['git', *args], text=True, stderr=subprocess.PIPE).strip()

        with tempfile.TemporaryDirectory() as directory, chdir(directory):
            git('init', '--bare', 'remote.git')
            Path('repo').mkdir()
            with chdir('repo'):
                git('init', '-b', 'main')
                git('config', 'user.name', 'Release test')
                git('config', 'user.email', 'test@example.invalid')
                Path('Cargo.toml').write_text('[package]\nname = "zcall"\nversion = "1.2.3"\n')
                Path('Cargo.lock').write_text('version = 3\n')
                for project in projects:
                    Path(project).mkdir()
                    Path(project, 'Cargo.lock').write_text(INDEPENDENT_LOCK)
                git('add', '.')
                git('commit', '-m', 'initial')
                git('remote', 'add', 'origin', '../remote.git')
                git('push', 'origin', 'main')
                version = '1.2.4' if change_root else '1.2.3'
                expected = {f'{project}/Cargo.lock' for project in projects}
                if change_root:
                    Path('Cargo.toml').write_text(f'[package]\nname = "zcall"\nversion = "{version}"\n')
                    Path('Cargo.lock').write_text('version = 3\n\n' + REGISTRY_ZCONV)
                    expected.update(('Cargo.toml', 'Cargo.lock'))
                for project in projects:
                    Path(project, 'Cargo.lock').write_text(INDEPENDENT_LOCK.replace(GIT_ZCONV, REGISTRY_ZCONV))
                # Exercise the workflow's actual shell commands with local Git remotes.
                # Only the final GitHub release API is replaced by a shell function.
                subprocess.run(['bash', '-euo', 'pipefail', '-c', 'gh() { test "$1" = release; }\n' + script],
                               env={**os.environ, 'RELEASE_VERSION': version}, check=True,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                self.assertEqual(set(git('diff', '--name-only', 'HEAD^', 'HEAD').splitlines()), expected)
                self.assertEqual(git('status', '--porcelain'), '')
                self.assertEqual(git('-C', '../remote.git', 'rev-parse', 'refs/heads/main'), git('rev-parse', 'HEAD'))
                self.assertEqual(git('-C', '../remote.git', 'rev-parse', f'refs/tags/v{version}^{{commit}}'),
                                 git('rev-parse', 'HEAD'))

    def test_release_commit_includes_both_existing_independent_locks(self):
        self.run_record_step(('comparison', 'fuzz'), change_root=True)

    def test_sidecar_only_change_is_committed_and_tagged(self):
        self.run_record_step(('fuzz',), change_root=False)

    def test_release_commit_succeeds_without_optional_projects(self):
        self.run_record_step((), change_root=True)


if __name__ == '__main__':
    unittest.main()
