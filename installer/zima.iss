; Zima installer (Inno Setup 6). Build the app first, then compile this script:
;
;   cargo build --release
;   ISCC installer\zima.iss          -> target\installer\Zima-Setup-<version>.exe
;
; Installs for the current user only (no admin prompt) into %LOCALAPPDATA%\Programs\Zima.
; Notes are never touched: they live in %APPDATA%\Zima (or a synced folder), which uninstalling keeps.

#define ExeFile SourcePath + "..\target\release\zima.exe"
#if !FileExists(ExeFile)
  #error target\release\zima.exe is missing. Run "cargo build --release" first.
#endif
; The version comes from Cargo.toml, built into zima.exe by build.rs.
#define AppVersion GetStringFileInfo(ExeFile, "ProductVersion")

[Setup]
; Never change AppId: it's how a new version finds and replaces the installed one.
AppId={{946E9F97-D1B7-4398-8F7B-72BD868C7EEB}
AppName=Zima
AppVersion={#AppVersion}
AppVerName=Zima {#AppVersion}
DefaultDirName={autopf}\Zima
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
WizardStyle=modern
SetupIconFile=..\ui\zima.ico
UninstallDisplayIcon={app}\zima.exe
UninstallDisplayName=Zima
OutputDir=..\target\installer
OutputBaseFilename=Zima-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
; Zima must be quit before its files are replaced or removed. It holds this name while running
; (src/instance.rs). Closing the window only hides it to the tray, so don't try to close it for the user.
AppMutex=ZimaRunning
CloseApplications=no

[Messages]
SetupAppRunningError=Zima is running.%n%nQuit it first: right-click the Zima icon next to the clock and choose Quit. Then click OK.
UninstallAppRunningError=Zima is running.%n%nQuit it first: right-click the Zima icon next to the clock and choose Quit. Then click OK.

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#ExeFile}"; DestDir: "{app}"; Flags: ignoreversion
; The bundled fonts are built into zima.exe; their licences (OFL) ship next to it.
Source: "..\fonts\OFL-*.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion

[Icons]
; The AppUserModelID must match APP_ID in src/system.rs: it's what lets notifications say "Zima".
Name: "{autoprograms}\Zima"; Filename: "{app}\zima.exe"; AppUserModelID: "Zima.Notes"
Name: "{autodesktop}\Zima"; Filename: "{app}\zima.exe"; AppUserModelID: "Zima.Notes"; Tasks: desktopicon

[Registry]
; "Launch at login" (Settings) is a Run entry pointing at zima.exe. If it's on, point it at the
; installed copy; uninstalling removes it either way.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Zima"; ValueData: """{app}\zima.exe"" --hidden"; Flags: uninsdeletevalue; Check: LaunchAtLoginOn
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Zima"; Flags: uninsdeletevalue dontcreatekey

[Run]
Filename: "{app}\zima.exe"; Description: "{cm:LaunchProgram,Zima}"; Flags: nowait postinstall skipifsilent

[Code]
function LaunchAtLoginOn: Boolean;
begin
  Result := RegValueExists(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Zima');
end;
