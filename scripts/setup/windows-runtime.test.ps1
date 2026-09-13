#requires -Version 5.1
param([switch]$NativeIntegration)
$ErrorActionPreference='Stop'
Set-StrictMode -Version 2.0
# Load only function definitions: never execute the installer while testing policy.
$tokens=$null;$errors=$null
$source=[IO.File]::ReadAllText((Join-Path $PSScriptRoot 'install.ps1')).Replace('@@PAYLOAD@@','')
$ast=[Management.Automation.Language.Parser]::ParseInput($source,[ref]$tokens,[ref]$errors)
if($errors.Count){throw ($errors|Out-String)}
foreach($name in @('Download','WindowsRuntimeVersion','EnsureWindowsRuntime')){
  $definition=$ast.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name},$true)
  if(-not $definition){throw "Missing function: $name"}
  . ([scriptblock]::Create($definition.Extent.Text))
}
$realVersion=${function:WindowsRuntimeVersion};$realDownload=${function:Download}
$directory=Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($directory)
try{
  function WindowsRuntimeVersion { $script:reads++; if($script:reads -eq 1){return $script:before};return $script:after }
  function Download { $script:downloads++; if($script:badHash){throw 'checksum mismatch'} }
  function Start-Process { $script:starts++; return @{ExitCode=$script:code} }
  foreach($case in @(
    @{Name='current';Before='14.51.36247.0';After='14.51.36247.0';Code=0;Downloads=0;Starts=0;Failure=$false},
    @{Name='newer';Before='14.60.0.0';After='14.60.0.0';Code=0;Downloads=0;Starts=0;Failure=$false},
    @{Name='missing';Before='0.0';After='14.51.36247.0';Code=0;Downloads=1;Starts=1;Failure=$false},
    @{Name='older';Before='14.40.0.0';After='14.51.36247.0';Code=0;Downloads=1;Starts=1;Failure=$false},
    @{Name='cancelled';Before='0.0';After='0.0';Code=1602;Downloads=1;Starts=1;Failure=$true},
    @{Name='false success';Before='0.0';After='0.0';Code=0;Downloads=1;Starts=1;Failure=$true},
    @{Name='restart';Before='0.0';After='14.51.36247.0';Code=3010;Downloads=1;Starts=1;Failure=$true},
    @{Name='newer installed concurrently';Before='0.0';After='14.60.0.0';Code=1638;Downloads=1;Starts=1;Failure=$false},
    @{Name='unverified download';Before='0.0';After='0.0';Code=0;Downloads=1;Starts=0;Failure=$true}
  )){
    $script:reads=0;$script:downloads=0;$script:starts=0
    $script:before=[version]$case.Before;$script:after=[version]$case.After;$script:code=$case.Code;$script:badHash=$case.Name -eq 'unverified download'
    $failed=$false;try{EnsureWindowsRuntime $directory}catch{$failed=$true}
    if($failed -ne $case.Failure -or $script:downloads -ne $case.Downloads -or $script:starts -ne $case.Starts){throw "Runtime policy failed: $($case.Name)"}
    Write-Host "PASS: $($case.Name)"
  }
  if($NativeIntegration){
    Remove-Item Function:Start-Process
    ${function:Download}=$realDownload
    # Exercise the real Microsoft download/installer even on a provisioned runner.
    # Only the first observation is overridden; completion reads the actual registry.
    $script:reads=0
    function WindowsRuntimeVersion { $script:reads++;if($script:reads -eq 1){return [version]'0.0'}; & $realVersion }
    $ReleaseDir=$null;$AppDir=$directory;$modelName='unused'
    EnsureWindowsRuntime $directory
    Write-Host "PASS: verified Microsoft runtime installer; installed $(& $realVersion)"
  }
}finally{Remove-Item -LiteralPath $directory -Recurse -Force}
