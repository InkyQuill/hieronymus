#requires -Version 7.4
[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$ReleaseDir,[string]$AppDir="$env:LOCALAPPDATA/Hieronymus/app",[string]$DataRoot="$env:APPDATA/Hieronymus",[string]$UnitDir,[switch]$NoActivate,[switch]$Uninstall)
$ErrorActionPreference='Stop'
if($Uninstall -and $NoActivate){throw '-Uninstall and -NoActivate cannot be combined'}
if (-not $IsWindows -or [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') { throw 'Windows x86_64 is required' }
$target='x86_64-pc-windows-msvc'
$ReleaseDir=[IO.Path]::GetFullPath($ReleaseDir);$AppDir=[IO.Path]::GetFullPath($AppDir);$DataRoot=[IO.Path]::GetFullPath($DataRoot)
function Regular([string]$path,[long]$limit) { $item=Get-Item -LiteralPath $path; if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $item.Length -gt $limit) {throw "Invalid bounded regular file: $path"} }
function CopyBounded([string]$source,[string]$destination,[long]$limit) {
 Regular $source $limit;$reader=[IO.File]::OpenRead($source);$writer=[IO.File]::Create($destination)
 try{$buffer=[byte[]]::new(65536);$count=0;while(($n=$reader.Read($buffer,0,$buffer.Length)) -gt 0){$count+=$n;if($count -gt $limit){throw 'Source changed beyond bootstrap bound'};$writer.Write($buffer,0,$n)}}finally{$reader.Dispose();$writer.Dispose()}
}
function Verify([string]$path,[string]$expected) {Regular $path 1073741824;if((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expected){throw "Checksum mismatch: $path"}}
function Unique($element) {if($element.ValueKind -eq 'Object'){$keys=[Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal);foreach($p in $element.EnumerateObject()){if(-not $keys.Add($p.Name)){throw 'Duplicate metadata key'};Unique $p.Value}}elseif($element.ValueKind -eq 'Array'){throw 'Arrays not permitted in release metadata'}}
$metadata=Join-Path $ReleaseDir "release-$target.json";Regular $metadata 65536
$doc=[Text.Json.JsonDocument]::Parse([IO.File]::ReadAllText($metadata));Unique $doc.RootElement
$m=$doc.RootElement.GetRawText()|ConvertFrom-Json -Depth 8
function Shape($obj,[string[]]$names){$actual=@($obj.PSObject.Properties.Name|Sort-Object);$expected=@($names|Sort-Object);if(($actual -join '
') -cne ($expected -join '
')){throw 'Unknown or missing metadata field'}}
Shape $m @('format_version','version','target','channel','platform','model','signature');Shape $m.platform @('archive','sha256');Shape $m.model @('archive','sha256','name','revision','members');Shape $m.model.members @('models/minilm/model.onnx','models/minilm/tokenizer.json','models/minilm/LICENSE','models/minilm/README.md')
if($doc.RootElement.GetProperty('format_version').GetRawText() -cne '2' -or $m.format_version -ne 2 -or $m.target -cne $target -or $null -ne $m.signature -or $m.version -cnotmatch '^\d+\.\d+\.\d+(?:[-+][A-Za-z0-9.-]+)?$' -or $m.channel -cnotin @('stable','dev')){throw 'Unsupported release identity'}
if($m.platform.archive -cne "hieronymus-$($m.version)-$target.zip" -or $m.platform.sha256 -cnotmatch '^[a-f0-9]{64}$' -or $m.model.sha256 -cnotmatch '^[a-f0-9]{64}$'){throw 'Invalid archive identity'}
$modelName='hieronymus-model-paraphrase-multilingual-MiniLM-L12-v2-e8f8c211226b894fcb81acc59f3b34ba3efd5f42.tar.gz'
if($m.model.archive -cne $modelName -or $m.model.name -cne 'paraphrase-multilingual-MiniLM-L12-v2' -or $m.model.revision -cne 'e8f8c211226b894fcb81acc59f3b34ba3efd5f42'){throw 'Unsupported common model'}
$pins=@{'models/minilm/model.onnx'='10f7a088420252b26caf819236ca2c9d2987afd0fc06fec7553b542a5655a05a';'models/minilm/tokenizer.json'='2c3387be76557bd40970cec13153b3bbf80407865484b209e655e5e4729076b8';'models/minilm/LICENSE'='cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30';'models/minilm/README.md'='1e98ea05b0de579fcaad3d625b62ea55647142ed674d5f5ebf1440e4bbbb6f23'}
foreach($name in $pins.Keys){if($m.model.members.$name -cne $pins[$name]){throw 'Model member pin mismatch'}}
$work=Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString('N'));[IO.Directory]::CreateDirectory($work)|Out-Null
try {
 CopyBounded $metadata (Join-Path $work "release-$target.json") 65536
 $platform=Join-Path $work $m.platform.archive;CopyBounded (Join-Path $ReleaseDir $m.platform.archive) $platform 1073741824;Verify $platform $m.platform.sha256
 $cache=Join-Path $AppDir "cache/models/$($m.model.sha256).tar.gz";if(Test-Path -LiteralPath $cache){Verify $cache $m.model.sha256}
 $source=Join-Path $ReleaseDir $modelName;if(-not(Test-Path -LiteralPath $source)){$source=$cache};Verify $source $m.model.sha256
 CopyBounded $source (Join-Path $work $modelName) 1073741824;Verify (Join-Path $work $modelName) $m.model.sha256
 $framing=[IO.File]::OpenRead($platform)
 try{$end=[byte[]]::new(22);if($framing.Length -lt 22){throw 'Truncated ZIP32'};[void]$framing.Seek(-22,[IO.SeekOrigin]::End);$framing.ReadExactly($end);if([BitConverter]::ToUInt32($end,0) -ne 0x06054b50 -or [BitConverter]::ToUInt16($end,4) -ne 0 -or [BitConverter]::ToUInt16($end,6) -ne 0 -or [BitConverter]::ToUInt16($end,8) -ne [BitConverter]::ToUInt16($end,10) -or [BitConverter]::ToUInt16($end,10) -gt 10000 -or [BitConverter]::ToUInt16($end,20) -ne 0 -or [long][BitConverter]::ToUInt32($end,12)+[BitConverter]::ToUInt32($end,16) -ne $framing.Length-22){throw 'Unsupported or unbounded ZIP32 framing'}}finally{$framing.Dispose()}
 $zip=[IO.Compression.ZipFile]::OpenRead($platform)
 try {
  $entries=@($zip.Entries|Where-Object {$_.FullName -ceq 'hiero.exe'})
  if($entries.Count -ne 1 -or $entries[0].Length -gt 536870912 -or (($entries[0].ExternalAttributes -shr 16) -band 0xF000) -notin @(0,0x8000)){throw 'Invalid bootstrap executable'}
  $inputStream=$entries[0].Open();$output=[IO.File]::Create((Join-Path $work 'hiero.exe'))
  try{$buffer=[byte[]]::new(65536);$count=0;while(($n=$inputStream.Read($buffer,0,$buffer.Length)) -gt 0){$count+=$n;if($count -gt 536870912){throw 'Expanded executable bound'};$output.Write($buffer,0,$n)}}finally{$inputStream.Dispose();$output.Dispose()}
 } finally {$zip.Dispose()}
 $cli=Join-Path $work 'hiero.exe'; & $cli release-verify --release-dir $work;if($LASTEXITCODE -ne 0){throw 'Complete release manifest verification failed'}
 if($Uninstall){$arguments=@('uninstall','--yes','--app-dir',$AppDir,'--data-root',$DataRoot)}else{$arguments=@('desktop-bootstrap','--release-dir',$work,'--app-dir',$AppDir,'--data-root',$DataRoot)}
 if($UnitDir){$arguments+=@('--unit-dir',[IO.Path]::GetFullPath($UnitDir))};if($NoActivate){$arguments+='--no-activate'}
 & $cli @arguments;if($LASTEXITCODE -ne 0){throw 'Desktop installation failed; inspect rollback/pending diagnostics'}
 if($Uninstall){Write-Output "Removed owned software; data preserved at $DataRoot";return}
 Write-Output "Installed Hieronymus $($m.version). Commands: $AppDir/bin. Signing: unsigned."
}finally{Remove-Item -LiteralPath $work -Recurse -Force;$doc.Dispose()}
