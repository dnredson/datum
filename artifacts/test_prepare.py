import json
from pathlib import Path
import tempfile
import unittest
import tomllib

from prepare import NODES, ROOT, prepare


class BundleTests(unittest.TestCase):
    def test_profiles_mounts_and_secrets_agree(self):
        with tempfile.TemporaryDirectory() as temp:
            for node in NODES:
                dest = Path(temp) / node
                result = prepare(node, dest)
                cfg = tomllib.loads((dest / 'agent.toml').read_text())
                self.assertEqual(cfg['agent']['node_id'], node)
                bindings = {b['secret_ref']: b['environment_variable'] for b in cfg['dispatch'].get('secret_bindings', [])}
                env_names = {line.split('=')[0] for line in (dest / 'node.env').read_text().splitlines()}
                self.assertEqual(set(bindings.values()), env_names)
                available = set()
                for name in result['artifacts']:
                    artifact = json.loads((dest / 'catalog' / (name + '.json')).read_text())
                    c = artifact['runtime']['container']
                    mapping = next(m for m in cfg['dispatch']['container_mappings'] if m['service_name'] == artifact['service_id'])
                    self.assertEqual(mapping['artifact'], c['image'])
                    self.assertEqual(mapping['container_name'], c['container_name'])
                    self.assertEqual(mapping['ports'], c['ports'])
                    self.assertTrue(set(artifact['control_plane']['secret_refs']) <= set(bindings))
                    self.assertTrue(set(artifact['control_plane']['dependencies']) <= available)
                    available.add(name)
                    for mount in c.get('configuration_mounts', []):
                        self.assertTrue((dest / 'config' / mount['source']).exists())
                before = {p.relative_to(dest): p.read_bytes() for p in dest.rglob('*') if p.is_file()}
                prepare(node, dest)
                self.assertEqual(before, {p.relative_to(dest): p.read_bytes() for p in dest.rglob('*') if p.is_file()})

    def test_preserves_modified_configuration_and_secrets(self):
        with tempfile.TemporaryDirectory() as temp:
            dest = Path(temp) / 'fog'
            prepare('fog-01', dest)
            secret = (dest / 'node.env').read_bytes()
            (dest / 'agent.toml').write_text('operator edit')
            with self.assertRaisesRegex(ValueError, 'overwrite'):
                prepare('fog-01', dest)
            self.assertEqual((dest / 'agent.toml').read_text(), 'operator edit')
            self.assertEqual((dest / 'node.env').read_bytes(), secret)

    def test_rejects_repository_destination(self):
        with self.assertRaisesRegex(ValueError, 'outside'):
            prepare('fog-01', ROOT / 'generated')

    def test_rejects_symlink_destination(self):
        with tempfile.TemporaryDirectory() as temp:
            dest = Path(temp) / 'link'
            actual = Path(temp) / 'actual'
            actual.mkdir()
            try:
                dest.symlink_to(actual, target_is_directory=True)
            except OSError:
                self.skipTest('symlink privilege unavailable')
            with self.assertRaisesRegex(ValueError, 'symlink'):
                prepare('fog-01', dest)


if __name__ == '__main__':
    unittest.main()
