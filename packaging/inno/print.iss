; Inno Setup script for Print.
;   iscc /DAppVersion=x.y.z packaging\inno\print.iss
; Expects target\release\print.exe to already be built (build.ps1 does both).

#define AppName      "Print"
#ifndef AppVersion
  #define AppVersion "0.0.1"
#endif
#define AppPublisher "kidchenko"
#define AppURL       "https://github.com/kidchenko/print"
#define AppExe       "print.exe"
; Data folder and Run value name. Must match the `name` given to traykit::init
; in src/main.rs: traykit's dirs and autostart derive both from it.
#define AppKey       "Print"

[Setup]
; Never reuse this GUID for another product: it is how Windows matches
; upgrades and uninstalls, and winget's ProductCode is derived from it.
AppId={{FD211DC9-7E58-4009-B506-FB298F39E513}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}/issues
AppUpdatesURL={#AppURL}/releases
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
LicenseFile=..\..\LICENSE
OutputDir=..\..\dist
OutputBaseFilename=Print-Setup-{#AppVersion}
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; The app itself never needs elevation; only Program Files does.
PrivilegesRequired=admin
; Close a running instance on upgrade/uninstall instead of hitting a locked exe.
CloseApplications=yes
CloseApplicationsFilter=*.exe
; [Tasks], [UninstallRun] and [Code] touch HKCU and the user's profile on
; purpose; see the notes there.
UsedUserAreasWarning=no
MinVersion=10.0

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
; Ticked by default: Win+Shift+S only reaches Print while it is running, so a
; Print that did not start at sign-in leaves the key opening Snipping Tool.
; Same HKCU value the app's own "Start at login" toggle uses.
Name: "autostart"; Description: "Start {#AppName} when I sign in";   GroupDescription: "Startup:"

[Files]
Source: "..\..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\Uninstall {#AppName}"; Filename: "{uninstallexe}"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run";   ValueType: string; ValueName: "{#AppKey}"; ValueData: """{app}\{#AppExe}""";   Tasks: autostart; Flags: uninsdeletevalue

[Run]
; As the signed-in user, not the elevated installer, so its keyboard hook is on
; that user's desktop and it writes that user's settings.
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName}";   Flags: nowait postinstall skipifsilent runasoriginaluser

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im {#AppExe}";   Flags: runhidden; RunOnceId: "KillPrint"

[UninstallDelete]
Type: files; Name: "{userappdata}\{#AppKey}\print.log"
Type: files; Name: "{userappdata}\{#AppKey}\print.log.1"

[Code]
// The app's own "Run at login" toggle writes the same HKCU value; clear it
// even if the installer task was never ticked. Under UAC an admin elevating
// their own account keeps the same HKCU, which is the normal case.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    RegDeleteValue(HKEY_CURRENT_USER,
      'Software\Microsoft\Windows\CurrentVersion\Run', '{#AppKey}');
    // Only succeeds once config.json is gone: settings survive a reinstall.
    RemoveDir(ExpandConstant('{userappdata}\{#AppKey}'));
    RemoveDir(ExpandConstant('{app}'));
  end;
end;
