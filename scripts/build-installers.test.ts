import {afterEach, expect, test} from 'bun:test';
import {mkdtempSync, mkdirSync, writeFileSync, readFileSync, readdirSync, rmSync, copyFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createHash} from 'node:crypto';
import {renderInstallers, buildInstallers} from './build-installers';
import {TARGETS, MODEL_NAME, MODEL_REVISION, modelMembers, modelArtifactName, platformArtifactName, type ReleaseV2} from './desktop-targets';
const roots: string[]=[];
const hash=(bytes:string|Buffer)=>createHash('sha256').update(bytes).digest('hex');
function temp(){const root=mkdtempSync(join(tmpdir(),'hiero-setup-test-'));roots.push(root);return root;}
afterEach(()=>{for(const root of roots.splice(0))rmSync(root,{recursive:true,force:true});});
function releases(platform:Buffer=Buffer.from('platform'),model:Buffer=Buffer.from('model')):ReleaseV2[]{return TARGETS.map(target=>({format_version:2,version:'0.9.0',target,channel:'stable',platform:{archive:platformArtifactName('0.9.0',target),sha256:hash(platform)},model:{archive:modelArtifactName(),sha256:hash(model),name:MODEL_NAME,revision:MODEL_REVISION,members:modelMembers()},signature:null}));}
test('standalone entries embed validated metadata and pin one release',()=>{
  const files=renderInstallers(releases());
  for(const text of Object.values(files)){expect(text).not.toContain('@@');expect(text).not.toContain('git clone');expect(text).toContain('releases/download/v0.9.0/');}
  expect(files['install-hieronymus.sh']).toContain('function fail()');
  expect(files['Install-Hieronymus.ps1']).toContain('#requires -Version 5.1');
  expect(files['Install-Hieronymus.ps1']).not.toContain('Text.Json');
});
test('mixed releases, duplicate targets and shell-injection metadata are rejected',()=>{
  const mixed=releases();mixed[1].version='0.9.1';expect(()=>renderInstallers(mixed)).toThrow();
  expect(()=>renderInstallers([releases()[0],...releases().slice(0,3)])).toThrow();
  const bad=releases();bad[0].platform.archive="x';touch /tmp/bad;#";expect(()=>renderInstallers(bad)).toThrow();
});
test('generator verifies all source archives before producing executable installers',async()=>{
  const root=temp(),out=join(root,'out');
  for(const r of releases()){writeFileSync(join(root,`release-${r.target}.json`),JSON.stringify(r));writeFileSync(join(root,r.platform.archive),'platform');writeFileSync(join(root,r.model.archive),'model');}
  writeFileSync(join(root,releases()[0].platform.archive),'wrong');
  await expect(buildInstallers(root,out)).rejects.toThrow('bytes mismatch');
  expect(readdirSync(root)).not.toContain('out');
});
const unix=test.skipIf(process.platform==='win32');
function fixture(){
  const root=temp(),release=join(root,'release'),bin=join(root,'tools'),scratch=join(root,'scratch'),app=join(root,'app with spaces'),data=join(root,'data with spaces');
  for(const dir of [release,bin,scratch])mkdirSync(dir);
  const payload=join(root,'payload');mkdirSync(payload);
  // Orchestration-only executable: native verification itself is tested by the real-package CI smoke.
  writeFileSync(join(payload,'hiero'),`#!/usr/bin/env bash\nset -eu\nprintf '%s\\n' "$*" >> "$SETUP_CALLS"\nif [ "$1" = desktop-bootstrap ]; then\n while [ "$#" -gt 0 ]; do if [ "$1" = --app-dir ]; then app="$2"; break; fi; shift; done\n mkdir -p "$app/bin"; cp "$0" "$app/bin/hiero"; chmod 700 "$app/bin/hiero"\nfi\n`,{mode:0o755});
  const archive=join(root,'fixture.tar.gz');const tar=Bun.spawnSync(['tar','-czf',archive,'-C',payload,'hiero']);if(tar.exitCode)throw Error(tar.stderr.toString());
  const records=releases(readFileSync(archive));
  for(const r of records){copyFileSync(archive,join(release,r.platform.archive));writeFileSync(join(release,r.model.archive),'model');}
  const script=join(root,'install.sh');writeFileSync(script,renderInstallers(records)['install-hieronymus.sh']);
  writeFileSync(join(bin,'curl'),`#!/usr/bin/env bash\nset -eu\nurl=''; out=''\nwhile [ "$#" -gt 0 ]; do case "$1" in https://*) url="$1"; shift;; --output) out="$2"; shift 2;; *) shift;; esac; done\nprintf '%s\\n' "$url" >> "$SETUP_REQUESTS"\n[ -z "\${SETUP_FAIL_DOWNLOAD:-}" ] || exit 22\ncp "$SETUP_RELEASE/\${url##*/}" "$out"\n`,{mode:0o755});
  const calls=join(root,'calls'),requests=join(root,'requests');
  const env={...process.env,PATH:`${bin}:${process.env.PATH}`,TMPDIR:scratch,SETUP_CALLS:calls,SETUP_REQUESTS:requests,SETUP_RELEASE:release};
  return {root,release,script,scratch,app,data,calls,requests,bin,env,records};
}
unix('one standalone file downloads app/model, installs into spaced paths and opens the author interface',()=>{
  const f=fixture();const run=Bun.spawnSync(['bash',f.script,'--app-dir',f.app,'--data-root',f.data,'--unit-dir',join(f.root,'units')],{env:f.env});
  expect(run.exitCode).toBe(0);expect(run.stderr.toString()).toBe('');
  expect(readFileSync(f.calls,'utf8')).toContain(`admin --data-root ${f.data}`);
  expect(readFileSync(f.requests,'utf8').trim().split('\n')).toHaveLength(2);
  expect(readdirSync(f.scratch)).toEqual([]);
});
unix('no-activate never opens or starts a server; offline installation needs no curl/network',()=>{
  const f=fixture();const run=Bun.spawnSync(['bash',f.script,'--release-dir',f.release,'--app-dir',f.app,'--data-root',f.data,'--no-activate'],{env:{...f.env,SETUP_FAIL_DOWNLOAD:'1'}});
  expect(run.exitCode).toBe(0);expect(readFileSync(f.calls,'utf8')).not.toContain('admin');expect(readdirSync(f.scratch)).toEqual([]);expect(readdirSync(f.root)).not.toContain('requests');
});
for(const kind of ['platform','model','network'] as const)unix(`${kind} failure cleans downloads and never executes a payload`,()=>{
  const f=fixture();if(kind==='platform')for(const r of f.records)writeFileSync(join(f.release,r.platform.archive),'broken');if(kind==='model')writeFileSync(join(f.release,modelArtifactName()),'broken');
  const run=Bun.spawnSync(['bash',f.script,'--app-dir',f.app,'--data-root',f.data],{env:{...f.env,...(kind==='network'?{SETUP_FAIL_DOWNLOAD:'1'}:{})}});
  expect(run.exitCode).not.toBe(0);expect(readdirSync(f.root)).not.toContain('calls');expect(readdirSync(f.scratch)).toEqual([]);
});
unix('unsupported computers fail before downloading anything',()=>{
  const f=fixture();writeFileSync(join(f.bin,'uname'),'#!/bin/sh\necho unsupported\n',{mode:0o755});
  const run=Bun.spawnSync(['bash',f.script],{env:f.env});expect(run.exitCode).toBe(2);expect(readdirSync(f.root)).not.toContain('requests');
});
unix('Rosetta Terminal selects Apple Silicon payload',()=>{
  const f=fixture();writeFileSync(join(f.bin,'uname'),'#!/bin/sh\nif [ "$1" = -s ]; then echo Darwin; else echo x86_64; fi\n',{mode:0o755});writeFileSync(join(f.bin,'sysctl'),'#!/bin/sh\necho 1\n',{mode:0o755});
  const run=Bun.spawnSync(['bash',f.script,'--app-dir',f.app,'--data-root',f.data,'--no-activate'],{env:f.env});expect(run.exitCode).toBe(0);expect(readFileSync(f.requests,'utf8')).toContain('aarch64-apple-darwin.tar.gz');
});
