import {expect,test} from "bun:test";
import {synchronizeVersion,selectedRun,buildAndPromote} from "./automatic-release";

test("workspace bump changes only owned versions and fails incomplete lockfiles",()=>{
 const manifest='[workspace.package]\nversion = "0.9.3"\n[dependencies]\nversion = "keep"\n';
 const lock=['hiero','hieronymus','hiero-desktop','foreign'].map(name=>`[[package]]\nname = "${name}"\nversion = "0.9.3"\n`).join('\n');
 const next=synchronizeVersion('0.10.0',manifest,lock);
 expect(next.cargo).toContain('version = "0.10.0"');
 expect(next.cargo).toContain('version = "keep"');
 expect(next.cargoLock).toContain('name = "foreign"\nversion = "0.9.3"');
 expect(next.cargoLock.match(/version = "0.10.0"/g)).toHaveLength(3);
 expect(()=>synchronizeVersion('invalid',manifest,lock)).toThrow();
 expect(()=>synchronizeVersion('0.10.0',manifest,'')).toThrow();
});
test("run selection binds source, workflow, event and unique dispatch identity",()=>{
 const sha='a'.repeat(40), request='fixture';
 const run={id:123,head_sha:sha,event:'workflow_dispatch',path:'.github/workflows/desktop-candidate.yml',display_title:`desktop-candidate / ${request}`};
 expect(selectedRun([{...run,head_sha:'b'.repeat(40)},run],sha,request)).toEqual(run);
 expect(selectedRun([{...run,event:'push'},{...run,path:'.github/workflows/other.yml'},{...run,display_title:'other'}],sha,request)).toBeUndefined();
});
test("only a successful matching candidate is promoted, with its immutable tag and run id",async()=>{
 const calls:string[][]=[],sha='a'.repeat(40);let request='';
 const invoke=(args:string[])=>{
  calls.push(args);
  if(args.includes('inputs[orchestration_id]'))throw Error('Unexpected argument shape');
  const identity=args.find(a=>a.startsWith('inputs[orchestration_id]='));if(identity)request=identity.split('=')[1];
  if(args[0]==='api' && args[1].includes('/runs?'))return JSON.stringify({workflow_runs:[{id:123,head_sha:sha,event:'workflow_dispatch',path:'.github/workflows/desktop-candidate.yml',display_title:`desktop-candidate / ${request}`,status:'completed',conclusion:'success'}]});
  return '';
 };
 expect(await buildAndPromote('InkyQuill/hieronymus','v0.10.0',sha,invoke,async()=>{},1)).toBe(123);
 expect(calls.at(-1)).toContain('inputs[candidate_run]=123');
 expect(calls.at(-1)).toContain('ref=v0.10.0');
});
test("failed candidates and timeouts cannot dispatch publication",async()=>{
 const sha='a'.repeat(40);let request='';let promotions=0;
 const invoke=(args:string[])=>{
  const identity=args.find(a=>a.startsWith('inputs[orchestration_id]='));if(identity)request=identity.split('=')[1];
  if(args.some(a=>a.includes('release-rust.yml')))promotions++;
  if(args[0]==='api' && args[1].includes('/runs?'))return JSON.stringify({workflow_runs:[{head_sha:sha,event:'workflow_dispatch',path:'.github/workflows/desktop-candidate.yml',display_title:`desktop-candidate / ${request}`,status:'completed',conclusion:'failure',html_url:'fixture-log'}]});
  return '';
 };
 await expect(buildAndPromote('InkyQuill/hieronymus','v0.10.0',sha,invoke,async()=>{},1)).rejects.toThrow('Candidate failed');
 expect(promotions).toBe(0);
 await expect(buildAndPromote('InkyQuill/hieronymus','v0.10.0',sha,()=>'{"workflow_runs":[]}',async()=>{},1)).rejects.toThrow('exceeded');
});

test("a candidate finishing after two hours is promoted without redispatch", async () => {
 const sha = 'a'.repeat(40);
 let request = '', polls = 0, builds = 0;
 const invoke = (args: string[]) => {
  const identity = args.find(a => a.startsWith('inputs[orchestration_id]='));
  if (identity) { request = identity.split('=')[1]; builds++; }
  if (args[0] === 'api' && args[1].includes('/runs?')) {
   polls++;
   return JSON.stringify({workflow_runs: [{id: 456, head_sha: sha, event: 'workflow_dispatch', path: '.github/workflows/desktop-candidate.yml', display_title: `desktop-candidate / ${request}`, status: polls >= 301 ? 'completed' : 'in_progress', conclusion: polls >= 301 ? 'success' : null}]});
  }
  return '';
 };
 expect(await buildAndPromote('InkyQuill/hieronymus', 'v0.10.0', sha, invoke, async () => {})).toBe(456);
 expect(builds).toBe(1);
});

test("release synchronization updates all installer links without touching unrelated versions", async () => {
 const {synchronizeReadme}=await import('./automatic-release');
 const original=await Bun.file(new URL('../README.md',import.meta.url)).text();
 const next=synchronizeReadme('0.42.3',original);
 expect(next).toContain('/v0.42.3/Hieronymus-0.42.3-Setup.exe');
 expect(next).toContain('/v0.42.3/Hieronymus-0.42.3.pkg');
 expect(next).toContain('/v0.42.3/install-hieronymus.sh');
 expect(next.split('## Develop and verify')[1]).toBe(original.split('## Develop and verify')[1]);
 expect(synchronizeReadme('0.42.3',next)).toBe(next);
 expect(()=>synchronizeReadme('../bad',original)).toThrow();
 expect(()=>synchronizeReadme('0.42.3',original.replace('install-hieronymus.sh','missing.sh'))).toThrow();
});
