; Built by .github/workflows/release.yml:
; iscc /DVersion=0.1.0 /DArch=x64 /DSource=<folder with x64\, x86\ and arm64\> /DOutput=<folder> packaging\vestigo.iss
;
; Arch=x64 makes one installer for x64 and x86: it carries both builds and installs the one that fits Windows.
; Arch=arm64 makes the ARM64 installer.

#ifndef Version
  #define Version "0.0.0"
#endif
#ifndef Arch
  #define Arch "x64"
#endif
#ifndef Source
  #define Source "..\build"
#endif
#ifndef Output
  #define Output "..\dist"
#endif

[Setup]
AppId={{3F6B1D0E-8C47-4A52-B9E1-6A0D2C7F4E15}
AppName=Vestigo
AppVersion={#Version}
AppVerName=Vestigo {#Version}
AppPublisher=Vestigo contributors
AppPublisherURL=https://github.com/ipavlin98/vestigo
AppSupportURL=https://github.com/ipavlin98/vestigo/issues
AppUpdatesURL=https://github.com/ipavlin98/vestigo/releases
DefaultDirName={autopf}\Vestigo
DefaultGroupName=Vestigo
DisableProgramGroupPage=yes
PrivilegesRequired=admin
#if Arch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
OutputBaseFilename=Vestigo-{#Version}-arm64-setup
#else
ArchitecturesAllowed=x86compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputBaseFilename=Vestigo-{#Version}-setup
#endif
MinVersion=10.0.17134
LicenseFile=..\LICENSE
OutputDir={#Output}
SetupIconFile=..\resources\icon.ico
UninstallDisplayIcon={app}\Vestigo.exe
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"

[CustomMessages]
en.RemoveData=Also delete your Vestigo settings, history and index?
ru.RemoveData=Удалить также настройки, историю и индекс Vestigo?

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
#if Arch == "arm64"
Source: "{#Source}\arm64\Vestigo.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\arm64\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion
#else
Source: "{#Source}\x64\Vestigo.exe"; DestDir: "{app}"; Check: Is64BitInstallMode; Flags: ignoreversion
Source: "{#Source}\x86\Vestigo.exe"; DestDir: "{app}"; Check: not Is64BitInstallMode; Flags: ignoreversion solidbreak
Source: "{#Source}\x64\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion solidbreak
#endif
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Vestigo"; Filename: "{app}\Vestigo.exe"
Name: "{autodesktop}\Vestigo"; Filename: "{app}\Vestigo.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Vestigo.exe"; Description: "{cm:LaunchProgram,Vestigo}"; Flags: nowait postinstall skipifsilent
Filename: "{app}\Vestigo.exe"; Flags: nowait; Check: Relaunch

[UninstallRun]
Filename: "{sys}\schtasks.exe"; Parameters: "/delete /tn Vestigo /f"; Flags: runhidden; RunOnceId: "StartWithWindows"

[Code]
procedure StopVestigo();
var
  Code: Integer;
  Exe: String;
begin
  Exe := ExpandConstant('{app}\Vestigo.exe');
  if not FileExists(Exe) then
    exit;
  Exec(Exe, '--quit', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(1000);
  Exec('powershell.exe',
    '-NoProfile -Command "Get-Process Vestigo -ErrorAction SilentlyContinue | Where-Object Path -eq ''' + Exe +
    ''' | Stop-Process -Force"',
    '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

function Relaunch(): Boolean;
begin
  Result := WizardSilent and (ExpandConstant('{param:relaunch|0}') = '1');
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopVestigo();
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  StopVestigo();
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent and
    (MsgBox(CustomMessage('RemoveData'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES) then
  begin
    DelTree(ExpandConstant('{userappdata}\Vestigo'), True, True, True);
    DelTree(ExpandConstant('{localappdata}\Vestigo'), True, True, True);
  end;
end;
