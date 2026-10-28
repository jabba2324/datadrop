; The Windows installer, built by Inno Setup in the release workflow:
;   iscc /DAppVersion=0.1.0 /DSourceDir=..\..\target\x86_64-pc-windows-msvc\release /DOutputDir=..\..\dist datadrop.iss
; It installs for the current user, or for everyone when run as an administrator.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{3D6F2A51-8C4E-4B7A-9E1D-5A0C7B2F9E64}
AppName=DataDrop
AppVersion={#AppVersion}
AppVerName=DataDrop {#AppVersion}
AppPublisher=The DataDrop authors
AppPublisherURL=https://github.com/jabba2324/datadrop
DefaultDirName={autopf}\DataDrop
DefaultGroupName=DataDrop
DisableProgramGroupPage=yes
OutputDir={#OutputDir}
OutputBaseFilename=DataDrop-v{#AppVersion}-x86_64-pc-windows-msvc-setup
SetupIconFile=..\icon.ico
UninstallDisplayIcon={app}\datadrop-gui.exe
LicenseFile=..\..\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ChangesEnvironment=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "addtopath"; Description: "Add the datadrop command line to PATH"

[Files]
Source: "{#SourceDir}\datadrop-gui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\datadrop.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\DataDrop"; Filename: "{app}\datadrop-gui.exe"
Name: "{autodesktop}\DataDrop"; Filename: "{app}\datadrop-gui.exe"; Tasks: desktopicon

[Registry]
; .sql files can be opened with DataDrop (it's offered, not made the default).
Root: HKA; Subkey: "Software\Classes\.sql\OpenWithProgids"; ValueType: string; ValueName: "DataDrop.sql"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\DataDrop.sql"; ValueType: string; ValueName: ""; ValueData: "SQL script"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\DataDrop.sql\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\datadrop-gui.exe"" ""%1"""

[Run]
Filename: "{app}\datadrop-gui.exe"; Description: "{cm:LaunchProgram,DataDrop}"; Flags: nowait postinstall skipifsilent

[Code]
function PathKey(): String;
begin
  if IsAdminInstallMode() then
    Result := 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
  else
    Result := 'Environment';
end;

function PathRoot(): Integer;
begin
  if IsAdminInstallMode() then Result := HKEY_LOCAL_MACHINE else Result := HKEY_CURRENT_USER;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Path: String;
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('addtopath') then
  begin
    if not RegQueryStringValue(PathRoot(), PathKey(), 'Path', Path) then Path := '';
    if Pos(';' + Uppercase(ExpandConstant('{app}')) + ';', ';' + Uppercase(Path) + ';') = 0 then
    begin
      if (Path <> '') and (Copy(Path, Length(Path), 1) <> ';') then Path := Path + ';';
      RegWriteExpandStringValue(PathRoot(), PathKey(), 'Path', Path + ExpandConstant('{app}'));
    end;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Path, App: String;
  P: Integer;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    if RegQueryStringValue(PathRoot(), PathKey(), 'Path', Path) then
    begin
      App := ExpandConstant('{app}');
      P := Pos(';' + Uppercase(App), ';' + Uppercase(Path));
      if P > 0 then
      begin
        Delete(Path, P, Length(App) + 1);
        RegWriteExpandStringValue(PathRoot(), PathKey(), 'Path', Path);
      end;
    end;
  end;
end;
