"""Offline failure-path tests: no helper process, network, credentials or writes."""
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


class FakeSession:
    def __init__(self, audit, mode):
        self.mode = mode
        self.created = False
        self.name = None
        self.song_added = False
        self.calls = []

    def close(self):
        pass

    def call(self, method, params=None, write=False):
        params = params or {}
        self.calls.append((method, params))
        if method == 'fetch_fav_albums':
            return {'albums': [{'id': 10}], 'total': 1, 'hasmore': 0}
        if method == 'fetch_fav_playlists':
            return {'playlists': [{'id': 20}], 'total': 1, 'hasmore': 0}
        if method == 'fetch_liked_songs':
            return {'likedSongs': {'tracks': [{'songId': 100}], 'total': 1}}
        if method == 'fetch_dislike_list':
            return {'retcode': 0, 'songs': [{'id': '100'}] if params.get('page', 1) == 1 else []}
        if method == 'fetch_created_playlists':
            rows = [{'dirid': 1, 'id': 11, 'title': 'original'}]
            if self.created:
                rows.append({'dirid': 2, 'id': 22, 'title': self.name})
            return {'playlists': rows}
        if method == 'query_songs':
            return {'tracks': [{'songId': 100, 'albumId': 10}]}
        if method == 'create_playlist':
            self.name = params['dirName']
            if self.mode == 'existing_id':
                return {'retCode': 0, 'dirid': 1, 'resultId': 11}
            self.created = True
            if self.mode == 'lost_create_reply':
                raise RuntimeError('transport parse failure after remote creation')
            return {'retCode': 0, 'dirid': 2, 'resultId': 22}
        if method == 'delete_playlist':
            if params['dirId'] != 2:
                raise AssertionError('attempted to delete original data')
            self.created = False
            return {'retCode': 0}
        if method == 'add_playlist_songs':
            self.song_added = True
            return {'success': True}
        if method == 'remove_playlist_songs':
            self.song_added = False
            return {'success': True}
        if method == 'fetch_playlist_tracks':
            return {'tracks': [{'songId': 100}] if self.song_added else []}
        if method == 'add_comment':
            return {'subcode': 1, 'resultId': '999'}
        if method == 'delete_comment':
            raise RuntimeError('comment cleanup failed')
        if method == 'search_playlists':
            return {'playlists': [{'id': 20}]}
        raise AssertionError(f'unexpected fake call: {method}')


class WriteCleanupTests(unittest.TestCase):
    def run_failure(self, mode):
        source = ROOT / 'scripts/port_write_smoke.py'
        namespace = {'__file__': str(source), '__name__': 'offline_write_smoke'}
        exec(compile(source.read_text(), str(source), 'exec'), namespace)
        sessions = []
        def factory(audit):
            session = FakeSession(audit, mode)
            sessions.append(session)
            return session
        namespace['Session'] = factory
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / 'report.json'
            with patch.object(sys, 'argv', ['test', '--execute-writes', '--report', str(report)]):
                with contextlib.redirect_stdout(io.StringIO()):
                    namespace['main']()
            result = json.loads(report.read_text())
        return sessions[0], result

    def test_late_reply_does_not_consume_cleanup_response(self):
        source = ROOT / 'scripts/port_write_smoke.py'
        namespace = {'__file__': str(source), '__name__': 'offline_write_smoke'}
        exec(compile(source.read_text(), str(source), 'exec'), namespace)
        class Process:
            stdin = io.StringIO()
            stdout = io.StringIO('{"id":"1","ok":true}\n{"id":"2","ok":true,"deleted":true}\n')
        class Selector:
            def register(self, *args):
                pass
            def select(self, timeout):
                return [(None, None)]
        with patch.object(namespace['subprocess'], 'Popen', return_value=Process()):
            with patch.object(namespace['selectors'], 'DefaultSelector', return_value=Selector()):
                session = namespace['Session']({'writes': []})
                session.sequence = 1  # Request 1 timed out; next call is cleanup request 2.
                self.assertTrue(session.call('delete_comment', {'cmId': 'test'}, write=True)['deleted'])

    def test_original_playlist_is_never_owned_by_failed_creation(self):
        session, result = self.run_failure('existing_id')
        self.assertFalse(any(method == 'delete_playlist' for method, _ in session.calls))
        self.assertEqual(result['groups'][0]['status'], 'failed')
        for name in ('album roundtrip', 'liked song roundtrip', 'dislike roundtrip'):
            statuses = [g['status'] for g in result['groups'] if g['name'] == name]
            self.assertEqual(statuses, ['skipped'])

    def test_lost_creation_reply_is_recovered_by_exact_name_readback(self):
        session, result = self.run_failure('lost_create_reply')
        self.assertFalse(session.created)
        deleted = [p['dirId'] for m, p in session.calls if m == 'delete_playlist']
        self.assertEqual(deleted, [2])
        self.assertTrue(result['restorations'][0]['verified'])

    def test_failed_comment_cleanup_does_not_prevent_playlist_cleanup(self):
        session, result = self.run_failure('comment_cleanup_error')
        self.assertFalse(session.created)
        self.assertTrue(any(m == 'delete_comment' for m, _ in session.calls))
        self.assertTrue(any(m == 'delete_playlist' for m, _ in session.calls))
        self.assertTrue(result['restorations'][0]['verified'])


if __name__ == '__main__':
    unittest.main()
