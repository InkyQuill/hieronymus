import {copyFileSync, chmodSync, mkdirSync, writeFileSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
function run(args:string[]){const r=Bun.spawnSync(args,{stdout:'inherit',stderr:'inherit'});if(r.exitCode)throw Error(`${args[0]} failed with exit ${r.exitCode}`);}
const [kind,input,out,version]=Bun.argv.slice(2);
if(!/^\d+\.\d+\.\d+$/.test(version??''))throw Error('stable version required');
const source=resolve(input),output=resolve(out);
mkdirSync(output,{recursive:true});
if(kind==='windows'){
  if(process.platform!=='win32')throw Error('Build the Windows installer on Windows');
  const compiler=process.env.NSIS_COMPILER??'C:\\Program Files (x86)\\NSIS\\makensis.exe';
  run([compiler,`/DVERSION=${version}`,`/DPAYLOAD=${join(source,'Install-Hieronymus.ps1')}`,`/DOUTPUT=${join(output,`Hieronymus-${version}-Setup.exe`)}`,fileURLToPath(new URL('./setup/windows.nsi',import.meta.url))]);
}else if(kind==='macos'){
  if(process.platform!=='darwin')throw Error('Build the macOS installer on macOS');
  const scripts=join(output,'pkg-scripts');mkdirSync(scripts);
  copyFileSync(join(source,'install-hieronymus.sh'),join(scripts,'install-hieronymus.sh'));
  copyFileSync(new URL('./setup/macos-postinstall.sh',import.meta.url),join(scripts,'postinstall'));
  chmodSync(join(scripts,'postinstall'),0o755);chmodSync(join(scripts,'install-hieronymus.sh'),0o755);
  run(['/usr/bin/pkgbuild','--nopayload','--scripts',scripts,'--identifier','net.inkyquill.hieronymus.setup','--version',version,join(output,'component.pkg')]);
  const resources=join(output,'resources');mkdirSync(resources);
  writeFileSync(join(resources,'welcome.html'),'<html><body><h1>Install Hieronymus</h1><p>Give your writing agent a memory.</p><p>This installer downloads the app and its memory model for the person signed in to this Mac. Keep your internet connection on; the download can take a few minutes.</p><p>Once installed, Hieronymus opens in your browser. Choose <b>Connect your agent</b> to add its MCP connection and skills, then continue writing in your agent.</p></body></html>');
  writeFileSync(join(output,'distribution.xml'),`<?xml version="1.0" encoding="utf-8"?>\n<installer-gui-script minSpecVersion="1"><title>Hieronymus</title><welcome file="welcome.html"/><options customize="never" require-scripts="true" hostArchitectures="arm64,x86_64"/><domains enable_anywhere="false" enable_currentUserHome="false" enable_localSystem="true"/><choices-outline><line choice="default"/></choices-outline><choice id="default" visible="false" title="Hieronymus"><pkg-ref id="net.inkyquill.hieronymus.setup"/></choice><pkg-ref id="net.inkyquill.hieronymus.setup" version="${version}">component.pkg</pkg-ref></installer-gui-script>\n`);
  run(['/usr/bin/productbuild','--distribution',join(output,'distribution.xml'),'--package-path',output,'--resources',resources,join(output,`Hieronymus-${version}.pkg`)]);
}else throw Error('expected windows or macos');
