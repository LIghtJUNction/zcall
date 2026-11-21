"""Release projection fixtures; Cargo and the registry are always mocked."""
from contextlib import chdir, redirect_stdout
from io import StringIO
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from unittest.mock import patch
from urllib.error import HTTPError

import release


REV = '0123456789abcdef0123456789abcdef01234567'
GIT = 'https://github.com/LIghtJUNction/zconv.git'
MANIFEST = '''[package]
name = "zcall"
version = "0.4.0"

[dependencies]
zconv = { default-features = false, git = "''' + GIT + '''" }
'''
LOCK = '''version = 4

[[package]]
name = "zcall"
version = "0.4.0"
dependencies = ["zconv"]

[[package]]
name = "zconv"
version = "0.4.0"
source = "git+''' + GIT + '#' + REV + '''"
'''
REGISTRY_LOCK = LOCK.replace(
    'source = "git+' + GIT + '#' + REV + '"',
    'source = "' + release.CRATES_IO_SOURCE + '"\nchecksum = "' + 'a' * 64 + '"')


class GitOnlyReleaseTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.enterContext(chdir(directory.name))
        self.root = Path.cwd()
        Path('Cargo.toml').write_text(MANIFEST)
        Path('Cargo.lock').write_text(LOCK)
        Path('fuzz').mkdir()
        Path('fuzz/Cargo.lock').write_text(LOCK)
        Path('.github').mkdir()
        Path('.github/release-request.json').write_text(json.dumps({
            'crate': 'zcall', 'version': '0.4.0', 'zconv': '=0.4.0', 'publish': True}))
        self.committed = LOCK
        self.resolved = {'name': 'zconv', 'version': '0.4.0',
                         'source': release.CRATES_IO_SOURCE, 'dependencies': []}
        self.output = self.enterContext(patch.object(
            release.subprocess, 'check_output', side_effect=self.check_output))
        self.run = self.enterContext(patch.object(
            release.subprocess, 'run', side_effect=self.run_cargo))
        self.registry = self.enterContext(patch.object(
            release, 'get_json', return_value={'version': {'num': '0.4.0', 'yanked': False}}))
        self.stage = self.enterContext(patch.object(
            release, 'create_release_stage', side_effect=self.create_stage, create=True))
        self.enterContext(redirect_stdout(StringIO()))

    def package(self):
        return tomllib.loads(Path('Cargo.toml').read_text())

    def check_output(self, command, **kwargs):
        if command == ['git', 'show', 'HEAD:Cargo.lock']:
            return self.committed
        if command == ['cargo', '+1.99.1', 'metadata', '--locked', '--format-version', '1']:
            self.assertEqual(Path.cwd(), self.root / 'target/release-stage')
            return json.dumps({'packages': [self.resolved]})
        self.fail(f'unexpected subprocess: {command}')

    def run_cargo(self, command, *, check):
        self.assertEqual(command, ['cargo', '+1.99.1', 'generate-lockfile'])
        self.assertTrue(check)
        self.assertEqual(Path.cwd(), self.root / 'target/release-stage')
        Path('Cargo.lock').write_text(REGISTRY_LOCK)

    def create_stage(self):
        stage = self.root / 'target/release-stage'
        stage.mkdir(parents=True)
        for name in ('Cargo.toml', 'Cargo.lock', 'fuzz/Cargo.lock'):
            dest = stage / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(self.root / name, dest)
        return stage

    def test_git_only_request_uses_committed_lock_version(self):
        before = Path('Cargo.toml').read_bytes()
        with patch.dict(os.environ, {'GITHUB_REF': 'refs/heads/main'}):
            release.request()
        self.assertEqual(Path('Cargo.toml').read_bytes(), before)
        self.registry.assert_not_called()
        self.run.assert_not_called()

    def test_git_only_dependency_validation_uses_committed_lock(self):
        self.assertEqual(release.validate_dependency(
            self.package(), {'num': '0.4.0', 'yanked': False}), '0.4.0')

    def test_legacy_exact_version_with_git_must_match_lock(self):
        package = self.package()
        package['dependencies']['zconv']['version'] = '=0.4.0'
        self.assertEqual(release.validate_dependency(package, {'num': '0.4.0'}), '0.4.0')
        for version in ('=0.3.0', '0.4', '*'):
            package['dependencies']['zconv']['version'] = version
            with self.subTest(version=version), self.assertRaises(ValueError):
                release.validate_dependency(package, {'num': '0.4.0'})

    def test_missing_ambiguous_registry_or_other_git_package_is_rejected(self):
        stanza = LOCK[LOCK.index('[[package]]\nname = "zconv"'):]
        invalid = (
            LOCK.replace(stanza, ''),
            LOCK + '\n' + stanza,
            LOCK + '\n' + stanza.replace('git+' + GIT + '#' + REV, release.CRATES_IO_SOURCE),
            REGISTRY_LOCK,
            LOCK.replace(GIT, 'https://github.com/other/zconv.git'),
            LOCK.replace('#' + REV, '#not-a-commit'),
            LOCK.replace('name = "zconv"\nversion = "0.4.0"', 'name = "zconv"\nversion = "latest"'),
            LOCK + 'dependencies = ["unexpected"]\n',
        )
        for lock in invalid:
            with self.subTest(lock=lock):
                self.committed = lock
                Path('Cargo.lock').write_text(lock)
                with self.assertRaises(ValueError):
                    release.dependency()
                self.registry.assert_not_called()
                self.stage.assert_not_called()
                self.run.assert_not_called()

    def test_changed_working_lock_cannot_override_committed_selection(self):
        for changed in (
            LOCK.replace(REV, 'f' * 40),
            LOCK.replace('name = "zconv"\nversion = "0.4.0"', 'name = "zconv"\nversion = "0.4.1"'),
            REGISTRY_LOCK,
        ):
            with self.subTest(changed=changed):
                Path('Cargo.lock').write_text(changed)
                with self.assertRaises(ValueError):
                    release.dependency()
                self.registry.assert_not_called()
                self.stage.assert_not_called()
        # A prepared local crate bump is allowed; zconv must stay identical.
        Path('Cargo.lock').write_text(LOCK.replace(
            'name = "zcall"\nversion = "0.4.0"', 'name = "zcall"\nversion = "0.4.1"'))
        self.assertEqual(release.validate_dependency(self.package(), {'num': '0.4.0'}), '0.4.0')

    def test_git_ref_and_package_alias_cannot_select_an_unrelated_lock(self):
        for change in ({'rev': 'f' * 40}, {'branch': 'other'}, {'tag': 'v0.4.0'},
                       {'package': 'other'}, {'git': 'https://github.com/other/zconv.git'}):
            with self.subTest(change=change):
                package = self.package()
                package['dependencies']['zconv'].update(change)
                with self.assertRaises(ValueError):
                    release.validate_dependency(package, {'num': '0.4.0'})

    def test_matching_explicit_git_revision_is_bound_to_full_lock_commit(self):
        package = self.package()
        package['dependencies']['zconv']['rev'] = REV
        self.committed = LOCK.replace('#' + REV, '?rev=' + REV + '#' + REV)
        Path('Cargo.lock').write_text(self.committed)
        self.assertEqual(release.validate_dependency(package, {'num': '0.4.0'}), '0.4.0')
        self.committed = self.committed.replace('#' + REV, '#' + 'f' * 40)
        Path('Cargo.lock').write_text(self.committed)
        with self.assertRaises(ValueError):
            release.validate_dependency(package, {'num': '0.4.0'})

    def test_uncommitted_or_absent_lock_is_not_a_fallback(self):
        self.output.side_effect = subprocess.CalledProcessError(128, ['git', 'show', 'HEAD:Cargo.lock'])
        with self.assertRaises(subprocess.CalledProcessError):
            release.dependency()
        self.registry.assert_not_called()
        self.stage.assert_not_called()
        self.run.assert_not_called()

    def test_registry_projection_is_isolated_and_keeps_development_bytes(self):
        before = {name: Path(name).read_bytes()
                  for name in ('Cargo.toml', 'Cargo.lock', 'fuzz/Cargo.lock')}
        release.dependency()
        self.assertEqual(Path.cwd(), self.root)
        self.assertEqual(before, {name: Path(name).read_bytes() for name in before})
        stage = self.root / 'target/release-stage'
        spec = tomllib.loads((stage / 'Cargo.toml').read_text())['dependencies']['zconv']
        self.assertEqual(spec, {'version': '=0.4.0', 'default-features': False})
        self.assertEqual((stage / 'Cargo.lock').read_text(), REGISTRY_LOCK)
        self.assertEqual((stage / 'fuzz/Cargo.lock').read_text(), REGISTRY_LOCK)
        self.registry.assert_called_once_with('https://crates.io/api/v1/crates/zconv/0.4.0')

    def test_registry_absence_yank_and_feature_guard_stop_before_staging(self):
        for result in ({'version': {'num': '0.4.0', 'yanked': True}},
                       {'version': {'num': '0.4.1'}}):
            with self.subTest(result=result):
                self.registry.return_value = result
                with self.assertRaises(ValueError):
                    release.dependency()
                self.stage.assert_not_called()
                self.run.assert_not_called()
        self.registry.side_effect = HTTPError('url', 404, 'missing', {}, None)
        with self.assertRaises(HTTPError):
            release.dependency()
        self.stage.assert_not_called()
        for change in ({'default-features': True}, {'features': ['std']}, {'path': '../zconv'}):
            package = self.package()
            package['dependencies']['zconv'].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                release.validate_dependency(package, {'num': '0.4.0'})

    def test_resolved_registry_identity_and_checksum_gates_are_not_relaxed(self):
        for change in ({'source': 'git+unverified'}, {'version': '0.4.1'},
                       {'dependencies': ['unexpected']}):
            with self.subTest(change=change):
                self.resolved.update(change)
                with self.assertRaises(ValueError):
                    release.dependency()
                self.assertEqual(Path('Cargo.toml').read_text(), MANIFEST)
                self.assertEqual(Path('fuzz/Cargo.lock').read_text(), LOCK)
                shutil.rmtree(self.root / 'target')
                self.resolved = {'name': 'zconv', 'version': '0.4.0',
                                 'source': release.CRATES_IO_SOURCE, 'dependencies': []}


class ReleaseStageTests(unittest.TestCase):
    def test_real_local_git_stage_preserves_development_and_prepared_version(self):
        with tempfile.TemporaryDirectory() as directory, chdir(directory):
            def git(*args):
                return subprocess.check_output(
                    ['git', *args], text=True, stderr=subprocess.PIPE).strip()

            git('init', '-b', 'main')
            git('config', 'user.name', 'Release fixture')
            git('config', 'user.email', 'fixture@example.invalid')
            Path('.gitignore').write_text('/target/\n')
            Path('Cargo.toml').write_text(MANIFEST)
            Path('Cargo.lock').write_text(LOCK)
            Path('src').mkdir()
            Path('src/lib.rs').write_text('// untouched source\n')
            git('add', '.')
            git('commit', '-m', 'fixture')
            head = git('rev-parse', 'HEAD')
            prepared = MANIFEST.replace('version = "0.4.0"', 'version = "0.4.1"')
            Path('Cargo.toml').write_text(prepared)
            Path('Cargo.lock').write_text(LOCK.replace(
                'name = "zcall"\nversion = "0.4.0"', 'name = "zcall"\nversion = "0.4.1"'))
            before = {name: Path(name).read_bytes() for name in ('Cargo.toml', 'Cargo.lock')}
            stage = release.create_release_stage()
            self.assertEqual(git('rev-parse', 'HEAD'), head)
            self.assertEqual(git('-C', str(stage), 'rev-parse', 'HEAD'), head)
            self.assertEqual(git('-C', str(stage), 'show', 'HEAD:Cargo.lock'), LOCK.rstrip('\n'))
            self.assertEqual((stage / 'Cargo.toml').read_text(), prepared)
            self.assertEqual((stage / 'src/lib.rs').read_bytes(), Path('src/lib.rs').read_bytes())
            self.assertEqual(before, {name: Path(name).read_bytes() for name in before})
            with self.assertRaises(ValueError):
                release.create_release_stage()
            self.assertEqual((stage / 'Cargo.toml').read_text(), prepared)



class ColdCacheWorkflowTests(unittest.TestCase):
    """Model only fetch/cache ordering; never execute a Cargo command."""
    def check_job(self, job):
        workflow = (Path(__file__).resolve().parents[1] / '.github/workflows/release.yml').read_text()
        section = re.search(r'(?ms)^  ' + job + r':\n(.*?)(?=^  [\w-]+:\n|\Z)', workflow)
        self.assertIsNotNone(section, job)
        commands = re.findall(r'(?m)^        run: ([^\n]+)$', section[1])
        warmed = False
        invoked = []

        def offline_update(command, *, check):
            invoked.append(command)
            self.assertEqual(command, ['cargo', '+1.99.1', 'update', '--workspace', '--offline'])
            self.assertTrue(check)
            if not warmed:
                raise subprocess.CalledProcessError(101, command, stderr='fixture: locked Git source is not cached')
            Path('Cargo.lock').write_text(LOCK.replace(
                'name = "zcall"\nversion = "0.4.0"', 'name = "zcall"\nversion = "0.4.1"'))

        with tempfile.TemporaryDirectory() as directory, chdir(directory):
            Path('Cargo.toml').write_text(MANIFEST)
            Path('Cargo.lock').write_text(LOCK)
            with patch.dict(os.environ, {'BUMP': 'patch', 'VERSION': '', 'RELEASE_TAG': '', 'GITHUB_OUTPUT': ''}), \
                    patch.object(release.subprocess, 'run', side_effect=offline_update), redirect_stdout(StringIO()):
                for command in commands:
                    if command == 'cargo +1.99.1 fetch --locked':
                        warmed = True  # Successful fetch is simulated, not executed.
                    if command == 'python3 scripts/release.py prepare':
                        release.prepare()
                        break
                else:
                    self.fail(f'{job} did not reach the real version preparation function')
            self.assertTrue(warmed)
            self.assertEqual(len(invoked), 1)
            self.assertEqual(tomllib.loads(Path('Cargo.toml').read_text())['package']['version'], '0.4.1')
            self.assertNotIn('version', tomllib.loads(Path('Cargo.toml').read_text())['dependencies']['zconv'])

    def test_verify_cold_cache_fetches_locked_source_before_offline_bump(self):
        self.check_job('verify')

    def test_publish_cold_cache_fetches_locked_source_before_offline_bump(self):
        self.check_job('publish')


if __name__ == '__main__':
    unittest.main()
