/** Render small, standalone launchers around checksum-bound released binaries. */
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { TARGETS, readReleaseV2, parseReleaseV2, type ReleaseV2 } from './desktop-targets';
import { digest } from './check-rust-release';

// PowerShell is an internal NSIS payload, never a user-facing Windows download.
export const INSTALLERS = ['install-hieronymus.sh', 'Install-Hieronymus.ps1'] as const;
const template = (name: string) => readFileSync(new URL(`./setup/${name}`, import.meta.url), 'utf8');
function heredoc(path: string, contents: string) {
  const marker = 'HIERONYMUS_EMBEDDED_FILE_END';
  if (contents.split('\n').includes(marker)) throw new Error('embedded delimiter collision');
  return `cat > "$work/${path}" <<'${marker}'\n${contents.trimEnd()}\n${marker}\n`;
}
export function renderInstallers(releases: readonly ReleaseV2[]) {
  releases = releases.map(r => parseReleaseV2(JSON.stringify(r), r.target));
  if (releases.length !== TARGETS.length || new Set(releases.map(r => r.target)).size !== TARGETS.length) throw new Error('all four target releases required');
  const version = releases[0].version;
  if (!/^\d+\.\d+\.\d+$/.test(version) || releases.some(r => r.version !== version || r.channel !== 'stable')) throw new Error('one stable release version required');
  const url = `https://github.com/InkyQuill/hieronymus/releases/download/v${version}`;
  const fill = (text: string, payload: string) => text.replaceAll('@@VERSION@@', version).replaceAll('@@RELEASE_URL@@', url).replace('@@PAYLOAD@@', payload);
  let payload = 'case "$target" in\n';
  for (const r of releases.filter(r => !r.target.includes('windows'))) {
    payload += `${r.target})\nplatform='${r.platform.archive}'; platform_hash='${r.platform.sha256}'\nmodel='${r.model.archive}'; model_hash='${r.model.sha256}'\n`;
    payload += heredoc(`release-${r.target}.json`, JSON.stringify(r)) + ';;\n';
  }
  payload += 'esac\n';
  payload += heredoc('offline.sh', readFileSync(new URL('./install-desktop.sh', import.meta.url), 'utf8'));
  payload += heredoc('desktop-metadata.awk', readFileSync(new URL('./desktop-metadata.awk', import.meta.url), 'utf8'));
  const windows = releases.find(r => r.target === 'x86_64-pc-windows-msvc')!;
  const ps = fill(template('install.ps1'), [
    `$metadataBase64='${Buffer.from(JSON.stringify(windows)).toString('base64')}'`,
    `$platformName='${windows.platform.archive}';$platformHash='${windows.platform.sha256}'`,
    `$modelName='${windows.model.archive}';$modelHash='${windows.model.sha256}'`,
  ].join('\n'));
  return { 'install-hieronymus.sh': fill(template('install.sh'), payload), 'Install-Hieronymus.ps1': ps };
}
export async function buildInstallers(directory: string, output: string) {
  const releases = TARGETS.map(target => readReleaseV2(join(directory, `release-${target}.json`), target));
  for (const r of releases) for (const artifact of [r.platform, r.model]) {
    if (await digest(join(directory, artifact.archive)) !== artifact.sha256) throw new Error(`release bytes mismatch: ${artifact.archive}`);
  }
  const files = renderInstallers(releases);
  mkdirSync(output, {recursive: true});
  for (const name of INSTALLERS) writeFileSync(join(output, name), files[name], {flag: 'wx', mode: 0o755});
  return [...INSTALLERS];
}
if (import.meta.main) await buildInstallers(Bun.argv[2], Bun.argv[3]);
