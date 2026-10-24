"""Offline publisher workflow tests. All git/gh/python commands are local fixture mocks."""
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
PWSH = shutil.which("pwsh.exe")
PS5 = shutil.which("powershell.exe")
NAME = "FreePort-v0.1.1-dev.1-windows-x64"
TAG = "v0.1.1-dev.1"
MOCK = r'''
import base64,json,os,sys
from pathlib import Path
p=Path(os.environ['FP_PUBLISH_FIXTURE_STATE'])
s=json.loads(p.read_text(encoding='utf-8'))
tool=sys.argv[1];a=sys.argv[2:];s['calls'].append([tool]+a)
def finish(value=None,code=0,error=None):
 p.write_text(json.dumps(s,ensure_ascii=True),encoding='ascii')
 if value is not None:print(value)
 if error:print(error,file=sys.stderr)
 raise SystemExit(code)
if tool=='python':finish('Fixture hooks ready')
if tool=='git':
 while a and a[0]=='-c':a=a[2:]
 if a==['remote','get-url','origin']:finish('https://github.com/suifei/freeport.git')
 if a==['branch','--show-current']:finish('main')
 if a in [['diff','--name-only'],['diff','--cached','--name-only'],['ls-files','--others','--exclude-standard']]:
  finish('scripts/publish-preview.ps1' if not s.get('committed') and a==['diff','--name-only'] else '')
 if a[:2]==['add','--']:s['staged']=not s.get('committed',False);finish()
 if a==['diff','--cached','--quiet']:finish(code=1 if s.get('staged') else 0)
 if a and a[0]=='commit':s['head']='c'*40;s['committed']=True;s['staged']=False;finish('Fixture commit')
 if a==['rev-parse','HEAD']:finish(s['head'])
 if a and a[0]=='show':finish('flynn <7343085+suifei@users.noreply.github.com>|flynn <7343085+suifei@users.noreply.github.com>')
 if a and a[0]=='push':s['remote_head']=s['head'];finish('Fixture push')
 if a[:2]==['diff','--quiet']:finish(code=1 if s.get('source_mismatch') else 0)
 finish(code=9,error='Unexpected fixture git call: '+repr(a))
if tool=='gh':
 if a and a[0]=='api':
  if '--jq' not in a or a[a.index('--jq')+1]!='@base64':finish(code=9,error='Unsafe raw JSON transport')
  path=a[1];obj=None
  if path=='user':obj={'login':'suifei','id':7343085,'name':'中文用户'}
  elif path.endswith('/commits/main'):obj={'sha':s['remote_head']}
  elif path.endswith('/git/ref/tags/'+s['tag']):
   if s.get('tag_commit'):obj={'object':{'type':'commit','sha':s['tag_commit']}}
  elif path.endswith('/releases/tags/'+s['tag']):
   if s.get('api_failure'):finish(code=1,error='gh: server error (HTTP 500)')
   obj=s.get('release')
  elif path.endswith('/releases/tags/v0.1.0-dev.1'):obj=s.get('old')
  else:finish(code=9,error='Unexpected fixture API: '+path)
  if obj is None:finish(code=1,error='gh: Not Found (HTTP 404)')
  finish(base64.b64encode(json.dumps(obj,ensure_ascii=False).encode('utf-8')).decode('ascii'))
 if a[:2]==['release','create']:
  if s.get('release') is not None:finish(code=9,error='Do not recreate a release')
  s['release']=s['template'];s['release']['assets']=s['files'].copy();s['tag_commit']=s['head'];finish(s['template']['html_url'])
 if a[:2]==['release','upload']:
  if '--clobber' in a:finish(code=9,error='Do not clobber')
  name=Path(a[3]).name
  if any(x['name']==name for x in s['release']['assets']):finish(code=9,error='Duplicate upload')
  s['release']['assets'].append(next(x for x in s['files'] if x['name']==name));finish('Fixture upload')
 if a[:2]==['release','delete-asset']:
  s['old']['assets']=[x for x in s['old']['assets'] if x['name']!=a[3]];finish('Fixture retirement')
 finish(code=9,error='Unexpected fixture gh call: '+repr(a))
finish(code=9,error='Unexpected fixture program')
'''


class PublisherTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="publisher-test-", dir=ROOT / ".build")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for folder in ("scripts", ".build", "dist", "mock-bin"):
            (self.root / folder).mkdir()
        shutil.copyfile(ROOT / "scripts/publish-preview.ps1", self.root / "scripts/publish-preview.ps1")
        (self.root / "Cargo.toml").write_bytes(b"fixture compiler input\n")
        archive = self.root / f"dist/{NAME}.zip"
        archive.write_bytes(b"OFFLINE TEST FIXTURE, NOT A DISTRIBUTABLE ZIP")
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        (self.root / f"dist/{NAME}.sha256").write_text(f"{digest}  {NAME}.zip\n", encoding="ascii")
        manifest = {"archive_sha256": digest, "files": [{"path": "Cargo.toml", "sha256": hashlib.sha256((self.root / 'Cargo.toml').read_bytes()).hexdigest()}]}
        (self.root / ".build/v011-source-fingerprint.json").write_text(json.dumps(manifest), encoding="ascii")
        files = []
        for suffix in ("zip", "sha256"):
            p = self.root / f"dist/{NAME}.{suffix}"
            files.append({"name": p.name, "size": p.stat().st_size, "digest": "sha256:" + hashlib.sha256(p.read_bytes()).hexdigest(), "state": "uploaded"})
        release = {"tag_name": TAG, "author": {"login": "suifei"}, "prerelease": True, "draft": False,
                   "html_url": f"https://github.com/suifei/freeport/releases/tag/{TAG}", "body": "开发预览。\n中文末尾。", "assets": files.copy()}
        self.state = {"tag": TAG, "head": "b" * 40, "remote_head": "b" * 40, "tag_commit": "b" * 40,
                      "files": files, "template": release.copy(), "release": release, "calls": [],
                      "old": {"assets": [{"name": "FreePort-dev-preview-v0.1.0.zip", "digest": "sha256:c1175f9302a79330da81b56df73a4d0bbd88f9ca5309c2253a48953a6792f193"}]}}
        self.state_path = self.root / "state.json"
        mock = self.root / "mock.py"
        mock.write_text(MOCK, encoding="utf-8")
        for tool in ("gh", "git", "python"):
            (self.root / f"mock-bin/{tool}.cmd").write_text(
                f'@echo off\r\n"{sys.executable}" "{mock}" {tool} %*\r\nexit /b %errorlevel%\r\n', encoding="utf-8")
        self.env = dict(os.environ, PATH=str(self.root / "mock-bin") + os.pathsep + os.environ["PATH"],
                        FP_PUBLISH_FIXTURE_STATE=str(self.state_path), POWERSHELL_TELEMETRY_OPTOUT="1")
        for key in ("GIT_AUTHOR_NAME", "GIT_COMMITTER_NAME", "GIT_AUTHOR_EMAIL", "GIT_COMMITTER_EMAIL"):
            self.env.pop(key, None)

    def run_publisher(self, expected=0):
        self.state_path.write_text(json.dumps(self.state, ensure_ascii=True), encoding="ascii")
        # Native mocks are the only permitted command resolution inside this fixture.
        path = str(self.root / "scripts/publish-preview.ps1").replace("'", "''")
        code = "[Console]::OutputEncoding=[Text.Encoding]::GetEncoding(936); "
        code += "foreach($name in @('git','gh','python')) { if ((Get-Command $name).Source -notlike '*mock-bin*') { throw 'Unsafe fixture command resolution' } }; "
        code += f"& '{path}' -RetireOldSourceBundle"
        result = subprocess.run([PWSH, "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", code],
                                env=self.env, capture_output=True, timeout=45)
        self.state = json.loads(self.state_path.read_text(encoding="ascii"))
        if expected == 0:
            self.assertEqual(result.returncode, 0, result.stdout.decode("utf-8", errors="replace") + result.stderr.decode("utf-8", errors="replace"))
        else:
            self.assertNotEqual(result.returncode, 0)
        return result

    def release_calls(self, action):
        return [c for c in self.state["calls"] if c[:3] == ["gh", "release", action]]

    def test_existing_release_resumes_and_repeated_run_is_idempotent(self):
        self.run_publisher()
        self.run_publisher()
        self.assertFalse(self.release_calls("create"))
        self.assertFalse(self.release_calls("upload"))
        self.assertEqual(len(self.release_calls("delete-asset")), 1)

    def test_missing_release_is_created_once(self):
        self.state["release"] = None
        self.state["tag_commit"] = None
        self.run_publisher()
        self.assertEqual(len(self.release_calls("create")), 1)
        self.assertFalse(self.release_calls("upload"))

    def test_partial_release_uploads_only_missing_asset(self):
        self.state["release"]["assets"] = self.state["files"][1:]
        self.run_publisher()
        self.assertFalse(self.release_calls("create"))
        self.assertEqual(len(self.release_calls("upload")), 1)

    def test_wrong_asset_is_never_overwritten_or_retired(self):
        self.state["release"]["assets"][0]["digest"] = "sha256:" + "0" * 64
        self.run_publisher(expected=1)
        self.assertFalse(self.release_calls("upload"))
        self.assertFalse(self.release_calls("delete-asset"))

    def test_tag_source_mismatch_blocks_retirement(self):
        self.state["source_mismatch"] = True
        self.run_publisher(expected=1)
        self.assertFalse(self.release_calls("delete-asset"))

    def test_server_error_is_not_treated_as_missing_release(self):
        self.state["api_failure"] = True
        self.run_publisher(expected=1)
        self.assertFalse(self.release_calls("create"))
        self.assertFalse(self.release_calls("delete-asset"))

    def test_utf8_transport_in_windows_powershell_and_pwsh(self):
        self.state_path.write_text(json.dumps(self.state, ensure_ascii=True), encoding="ascii")
        expected = base64.b64encode(self.state["release"]["body"].encode("utf-8")).decode("ascii")
        code = "[Console]::OutputEncoding=[Text.Encoding]::GetEncoding(936); "
        code += "$wire=@(& gh api 'repos/suifei/freeport/releases/tags/" + TAG + "' --jq '@base64'); "
        code += "$utf8=[Text.UTF8Encoding]::new($false,$true); "
        code += "$json=$utf8.GetString([Convert]::FromBase64String(($wire -join ''))); "
        code += "$actual=(ConvertFrom-Json -InputObject $json).body; "
        code += "$expected=$utf8.GetString([Convert]::FromBase64String('" + expected + "')); "
        code += "if($actual -cne $expected){throw 'UTF-8 transport corrupted Unicode JSON'}; 'PASS'"
        for engine in (PS5, PWSH):
            with self.subTest(engine=engine):
                result = subprocess.run([engine, "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", code],
                                        env=self.env, capture_output=True, timeout=30)
                self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", errors="replace"))
                self.assertIn(b"PASS", result.stdout)


if __name__ == "__main__":
    if not PWSH or not PS5:
        raise SystemExit("Windows PowerShell and pwsh are required")
    (ROOT / ".build").mkdir(exist_ok=True)
    unittest.main(verbosity=2)
