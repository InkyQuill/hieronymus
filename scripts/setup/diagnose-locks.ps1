$ErrorActionPreference='Stop'
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
using System.Text;
public static class Locks {
 [StructLayout(LayoutKind.Sequential)] public struct Unique { public uint Id; public FILETIME Started; }
 [StructLayout(LayoutKind.Sequential,CharSet=CharSet.Unicode)] public struct Info {
  public Unique Process;
  [MarshalAs(UnmanagedType.ByValTStr,SizeConst=256)] public string Name;
  [MarshalAs(UnmanagedType.ByValTStr,SizeConst=64)] public string Service;
  public uint Type, Status, Session;
  [MarshalAs(UnmanagedType.Bool)] public bool Restartable;
 }
 [DllImport("rstrtmgr.dll",CharSet=CharSet.Unicode)] static extern int RmStartSession(out uint session,uint flags,StringBuilder key);
 [DllImport("rstrtmgr.dll",CharSet=CharSet.Unicode)] static extern int RmRegisterResources(uint session,uint files,string[] paths,uint processes,IntPtr apps,uint services,IntPtr names);
 [DllImport("rstrtmgr.dll")] static extern int RmGetList(uint session,out uint needed,ref uint count,[In,Out] Info[] items,out uint reasons);
 [DllImport("rstrtmgr.dll")] static extern int RmEndSession(uint session);
 public static string Query(string[] paths) {
  uint session; int code=RmStartSession(out session,0,new StringBuilder(33));
  if(code!=0)return "start="+code;
  try {
   code=RmRegisterResources(session,(uint)paths.Length,paths,0,IntPtr.Zero,0,IntPtr.Zero);
   if(code!=0)return "register="+code;
   uint needed,count=128,reasons; var items=new Info[count];
   code=RmGetList(session,out needed,ref count,items,out reasons);
   if(code!=0)return "query="+code+" needed="+needed;
   var result=new StringBuilder("reason="+reasons);
   for(int i=0;i<count;i++)result.Append(" PID="+items[i].Process.Id+" name="+items[i].Name+" type="+items[i].Type);
   return result.ToString();
  } finally { RmEndSession(session); }
 }
}
'@
New-Item -ItemType Directory -Path diagnostics -Force | Out-Null
$app=Join-Path $env:RUNNER_TEMP 'Hiero lock diagnostic'
$data=Join-Path $env:RUNNER_TEMP 'Hiero diagnostic data'
$units=Join-Path $env:RUNNER_TEMP 'Hiero diagnostic units'
$script=(Resolve-Path installers/Install-Hieronymus.ps1).Path
$payload=(Resolve-Path payloads).Path
$args='-NoProfile -ExecutionPolicy Bypass -File "{0}" -ReleaseDir "{1}" -AppDir "{2}" -DataRoot "{3}" -UnitDir "{4}" -NoActivate' -f $script,$payload,$app,$data,$units
$child=Start-Process "$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" -ArgumentList $args -PassThru -RedirectStandardOutput "$PWD/diagnostics/install-out.log" -RedirectStandardError "$PWD/diagnostics/install-err.log"
$deadline=(Get-Date).AddMinutes(6)
$seen=@{}
while(-not $child.HasExited -and (Get-Date) -lt $deadline) {
 $stage=Join-Path $app 'versions/.staging-0.9.1'
 if(Test-Path $stage) {
  $files=@(Get-ChildItem $stage -Recurse -File -ErrorAction SilentlyContinue | Where-Object {$_.Extension -in '.exe','.dll'} | ForEach-Object {$_.FullName})
  if($files.Count) {
   $observation=[Locks]::Query([string[]]$files)
   if(-not $seen.ContainsKey($observation)) {
    $seen[$observation]=$true
    "$(Get-Date -Format o) $observation" | Tee-Object -FilePath diagnostics/locks.log -Append
   }
  }
  Get-Process -Name 'hiero*' -ErrorAction SilentlyContinue | ForEach-Object {
   $p=$_
   try { $modules=@($p.Modules | Where-Object {$_.FileName.StartsWith($stage)} | ForEach-Object {$_.FileName}) } catch {$modules=@('unavailable')}
   $observation="PID=$($p.Id) exe=$($p.Path) stagedModules=$($modules -join ',')"
   if(-not $seen.ContainsKey($observation)) {
    $seen[$observation]=$true
    "$(Get-Date -Format o) $observation" | Tee-Object -FilePath diagnostics/locks.log -Append
   }
  }
 }
 Start-Sleep -Milliseconds 250
 $child.Refresh()
}
if(-not $child.HasExited) { throw 'Diagnostic installer exceeded six minutes' }
$child.WaitForExit()
"Installer exit=$($child.ExitCode)" | Tee-Object -FilePath diagnostics/result.log
Get-Content diagnostics/install-out.log,diagnostics/install-err.log
if(Test-Path $app) { Get-ChildItem $app -Recurse | Select-Object FullName,Attributes | ConvertTo-Json | Set-Content diagnostics/remaining-files.json }
if($child.ExitCode -ne 0) { throw 'Installer reproduced failure; see lock observations' }
