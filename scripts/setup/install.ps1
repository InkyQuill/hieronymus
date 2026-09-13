#requires -Version 5.1
[CmdletBinding()]
param([string]$AppDir="$env:LOCALAPPDATA/Hieronymus/app",[string]$DataRoot="$env:APPDATA/Hieronymus",[string]$UnitDir,[string]$ReleaseDir,[string]$LogPath,[switch]$NoActivate,[switch]$NoOpen)
$ErrorActionPreference='Stop'
Set-StrictMode -Version 2.0
if($LogPath){[void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($LogPath)));Start-Transcript -LiteralPath $LogPath -Force|Out-Null}
if($env:OS -ne 'Windows_NT' -or -not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -eq 'ARM64' -or $env:PROCESSOR_ARCHITEW6432 -eq 'ARM64'){throw 'This installer needs Windows x86_64.'}
$AppDir=[IO.Path]::GetFullPath($AppDir);$DataRoot=[IO.Path]::GetFullPath($DataRoot)
@@PAYLOAD@@
function Download([string]$name,[string]$hash,[string]$destination,[string]$sourceUrl){
  $cached=$null
  if($ReleaseDir -and -not $sourceUrl){$cached=Join-Path $ReleaseDir $name}
  elseif([IO.File]::Exists((Join-Path $AppDir "cache/downloads/$name"))){$cached=Join-Path $AppDir "cache/downloads/$name"}
  elseif($name -eq $modelName -and [IO.File]::Exists((Join-Path $AppDir "cache/models/$hash.tar.gz"))){$cached=Join-Path $AppDir "cache/models/$hash.tar.gz"}
  if($cached){
    $file=Get-Item -LiteralPath $cached
    if($file.PSIsContainer -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $file.Length -gt 1073741824){throw 'Invalid offline download.'}
    [IO.File]::Copy($file.FullName,$destination)
  }else{
    $url=if($sourceUrl){[Uri]$sourceUrl}else{[Uri]("@@RELEASE_URL@@/"+$name)}
    $response=$null
    [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12
    for($redirect=0;$redirect -le 5;$redirect++){
      if($url.Scheme -ne 'https' -or $url.UserInfo){throw 'Unsafe download address.'}
      $request=[Net.HttpWebRequest]::Create($url);$request.AllowAutoRedirect=$false;$request.Timeout=30000;$request.ReadWriteTimeout=30000
      $response=$request.GetResponse()
      if([int]$response.StatusCode -ge 300 -and [int]$response.StatusCode -lt 400){
        $location=$response.Headers['Location'];$response.Dispose();$response=$null
        if(-not $location -or $redirect -eq 5){throw 'Could not resolve the download.'}
        $url=[Uri]::new($url,$location);continue
      }
      break
    }
    if(-not $response){throw 'Download did not return a file.'}
    try{
      if($response.ContentLength -gt 1073741824){throw 'Download is larger than expected.'}
      $reader=$response.GetResponseStream();$writer=[IO.File]::Create($destination)
      try{
        $buffer=New-Object byte[] 65536;$count=0L;$timer=[Diagnostics.Stopwatch]::StartNew()
        while(($n=$reader.Read($buffer,0,$buffer.Length)) -gt 0){
          $count+=$n;if($count -gt 1073741824 -or $timer.Elapsed.TotalMinutes -gt 15){throw 'Download exceeded its size or time limit.'}
          $writer.Write($buffer,0,$n)
        }
      }finally{$reader.Dispose();$writer.Dispose()}
    }finally{$response.Dispose()}
  }
  $reader=[IO.File]::OpenRead($destination);$sha=[Security.Cryptography.SHA256]::Create()
  try{$actual=[BitConverter]::ToString($sha.ComputeHash($reader)).Replace('-','').ToLowerInvariant()}finally{$reader.Dispose();$sha.Dispose()}
  if((Get-Item -LiteralPath $destination).Length -gt 1073741824 -or $actual -cne $hash){throw 'The download could not be verified. Nothing was installed. Please try again.'}
}
function WindowsRuntimeVersion {
  $registry=[Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine,[Microsoft.Win32.RegistryView]::Registry64)
  try{
    $key=$registry.OpenSubKey('SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64')
    if(-not $key){return [version]'0.0.0.0'}
    try{
      if($key.GetValue('Installed',0) -ne 1){return [version]'0.0.0.0'}
      return [version]::new([int]$key.GetValue('Major',0),[int]$key.GetValue('Minor',0),[int]$key.GetValue('Bld',0),[int]$key.GetValue('Rbld',0))
    }finally{$key.Dispose()}
  }finally{$registry.Dispose()}
}
function EnsureWindowsRuntime([string]$directory){
  $required=[version]'14.51.36247.0'
  if((WindowsRuntimeVersion) -ge $required){return}
  Write-Host 'Installing the Microsoft Windows runtime. Windows may ask you to allow this prerequisite.'
  $installer=Join-Path $directory 'VC_redist.x64.exe'
  Download 'VC_redist.x64.exe' '843068991daaa1f73ad9f6239bce4d0f6a07a51f18c37ea2a867e9beca71295c' $installer 'https://download.visualstudio.microsoft.com/download/pr/ebdab8e5-1d7b-4d9f-a11b-cbb1720c3b12/843068991DAAA1F73AD9F6239BCE4D0F6A07A51F18C37EA2A867E9BECA71295C/VC_redist.x64.exe'
  $result=Start-Process -FilePath $installer -ArgumentList '/install /quiet /norestart' -Verb RunAs -Wait -PassThru
  if($result.ExitCode -notin @(0,3010,1638) -or (WindowsRuntimeVersion) -lt $required){throw 'The Microsoft Windows runtime could not be installed. Run Setup again and allow the prerequisite when Windows asks.'}
  if($result.ExitCode -eq 3010){throw 'The Microsoft Windows runtime needs a restart. Restart Windows, then run Hieronymus Setup again. Your project data has not been changed.'}
}
function CopyStream($reader,$writer,[long]$limit){
  $buffer=New-Object byte[] 65536;$count=0L
  while(($n=$reader.Read($buffer,0,$buffer.Length)) -gt 0){$count+=$n;if($count -gt $limit){throw 'Expanded executable is too large.'};$writer.Write($buffer,0,$n)}
}
$work=Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($work)
try{
  Write-Host 'Installing Hieronymus @@VERSION@@...'
  EnsureWindowsRuntime $work
  [IO.File]::WriteAllBytes((Join-Path $work 'release-x86_64-pc-windows-msvc.json'),[Convert]::FromBase64String($metadataBase64))
  Write-Host 'Downloading the app...'
  $platform=Join-Path $work $platformName;Download $platformName $platformHash $platform
  Write-Host 'Downloading the memory model (this can take a few minutes)...'
  Download $modelName $modelHash (Join-Path $work $modelName)
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $zip=[IO.Compression.ZipFile]::OpenRead($platform)
  try{
    $entries=@($zip.Entries|Where-Object {$_.FullName -ceq 'hiero.exe'})
    if($zip.Entries.Count -gt 10000 -or $entries.Count -ne 1 -or $entries[0].Length -gt 536870912 -or (($entries[0].ExternalAttributes -shr 16) -band 0xF000) -notin @(0,0x8000)){throw 'Invalid bootstrap executable.'}
    $reader=$entries[0].Open();$writer=[IO.File]::Create((Join-Path $work 'hiero.exe'))
    try{CopyStream $reader $writer 536870912}finally{$reader.Dispose();$writer.Dispose()}
  }finally{$zip.Dispose()}
  Write-Host 'Verifying and installing...'
  $cli=Join-Path $work 'hiero.exe'
  & $cli release-verify --release-dir $work *> (Join-Path $work 'verify.log')
  if($LASTEXITCODE -ne 0){Get-Content -LiteralPath (Join-Path $work 'verify.log');throw 'The complete release could not be verified.'}
  $arguments=@('desktop-bootstrap','--release-dir',$work,'--app-dir',$AppDir,'--data-root',$DataRoot)
  if($UnitDir){$arguments+=@('--unit-dir',[IO.Path]::GetFullPath($UnitDir))};if($NoActivate){$arguments+='--no-activate'}
  & $cli @arguments *> (Join-Path $work 'install.log')
  if($LASTEXITCODE -ne 0){Get-Content -LiteralPath (Join-Path $work 'install.log');throw 'Installation needs attention. Your existing project data is preserved.'}
  # Windows cannot remove the executable that is currently running. Keep the
  # verified real CLI outside the managed app directory for the native uninstaller.
  $uninstallRoot=Join-Path $env:LOCALAPPDATA 'Hieronymus'
  [void][IO.Directory]::CreateDirectory($uninstallRoot)
  [IO.File]::Copy($cli,(Join-Path $uninstallRoot 'uninstall-hiero.exe'),$true)
  Write-Host 'Hieronymus is installed.'
  if(-not $NoActivate -and -not $NoOpen){
    Write-Host 'Opening Hieronymus. Choose "Connect your agent" to finish setup.'
    $opened=Start-Process -FilePath (Join-Path $AppDir 'bin/hiero.exe') -ArgumentList @('admin','--data-root',('"'+$DataRoot+'"')) -Wait -PassThru
    if($opened.ExitCode -ne 0){Get-Content -LiteralPath (Join-Path $work 'install.log');Write-Warning 'Could not open the web interface automatically. Use the Hieronymus tray icon to open it.'}
  }
}finally{Remove-Item -LiteralPath $work -Recurse -Force;if($LogPath){Stop-Transcript|Out-Null}}
