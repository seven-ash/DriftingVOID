; SPDX-License-Identifier: GPL-3.0-or-later
; Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.
;
; Drifting VOID installer (Inno Setup 6.7). Built by dist\package.ps1 from
; the packaged folder:
;   ISCC /DAppVer=1.0.1.R /DFileVer=1.0.1.0 /DSrc=<package folder> /DOut=<dir> dist\inno\DriftingVOID.iss
; Per user, no administrator rights; the same folder as the PowerShell setup
; (%LOCALAPPDATA%\Programs\DriftingVOID), which it replaces. FFmpeg is not
; inside: an optional task downloads the public build from GitHub during
; setup (ffmpeg.ps1 picks it; Inno Setup downloads and checks its SHA-256).

#ifndef AppVer
  #define AppVer "0.0.0-dev"
#endif
#ifndef FileVer
  #define FileVer "0.0.0.0"
#endif
#ifndef Src
  #error Pass /DSrc=<package folder>
#endif
#ifndef Out
  #define Out "."
#endif

#define AppName "Drifting VOID"
#define ExeName "DriftingVOID.exe"
#define Repo "https://github.com/seven-ash/DriftingVOID"

[Setup]
AppId={{6B0E7C2A-4D1F-4E8B-9A57-1C3D5F70D1A0}
AppName={#AppName}
AppVersion={#AppVer}
AppVerName={#AppName} {#AppVer}
AppPublisher=Ash (seven-ash)
AppPublisherURL={#Repo}
AppSupportURL={#Repo}/issues
AppUpdatesURL={#Repo}/releases
AppCopyright=Copyright (C) 2026 Ash (seven-ash). GPL-3.0-or-later.
VersionInfoVersion={#FileVer}
VersionInfoProductVersion={#FileVer}
VersionInfoProductTextVersion={#AppVer}
VersionInfoDescription={#AppName} Setup
PrivilegesRequired=lowest
DefaultDirName={autopf}\DriftingVOID
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
; the welcome page carries the wizard image
DisableWelcomePage=no
ChangesAssociations=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#Out}
OutputBaseFilename=DriftingVOID-{#AppVer}-Setup
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\{#ExeName}
UninstallDisplayName={#AppName}
Compression=lzma2/max
SolidCompression=yes
; the night of the app and its icon, dark always (the VOID); the welcome
; and finish image from art.py
WizardStyle=modern dark includetitlebar
WizardBackColor=#18171F
WizardImageBackColor=#0D0C13
WizardSmallImageBackColor=#18171F
WizardImageFile=img\wizard-164x314.png,img\wizard-205x393.png,img\wizard-246x471.png,img\wizard-287x550.png,img\wizard-328x628.png
WizardSmallImageFile=img\small-55.png,img\small-69.png,img\small-83.png,img\small-96.png,img\small-110.png
ShowLanguageDialog=auto
LanguageDetectionMethod=uilanguage
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"

[CustomMessages]
english.ManualLink={#AppName} manual
korean.ManualLink={#AppName} 사용 설명서
japanese.ManualLink={#AppName} マニュアル
english.FileType={#AppName} project
korean.FileType={#AppName} 프로젝트
japanese.FileType={#AppName} プロジェクト
english.FfTask=Download FFmpeg for video and sound (about 80 MB, from GitHub)
korean.FfTask=영상·소리에 쓰는 FFmpeg 내려받기 (약 80 MB, GitHub)
japanese.FfTask=動画と音声用の FFmpeg をダウンロード (約 80 MB、GitHub)
english.FfGroup=FFmpeg (a separate program under the GPL, from BtbN/FFmpeg-Builds):
korean.FfGroup=FFmpeg (GPL 라이선스의 별도 프로그램, BtbN/FFmpeg-Builds):
japanese.FfGroup=FFmpeg (GPL ライセンスの別のプログラム、BtbN/FFmpeg-Builds):
english.FfFailed=FFmpeg could not be set up (%1). Drifting VOID is installed; video and sound need FFmpeg; run this setup again later to fetch it.
korean.FfFailed=FFmpeg를 설치하지 못했습니다 (%1). Drifting VOID는 설치되었으며, 영상·소리에는 FFmpeg가 필요합니다. 나중에 이 설치를 다시 실행하면 받을 수 있습니다.
japanese.FfFailed=FFmpeg を設定できませんでした (%1)。Drifting VOID はインストール済みです。動画と音声には FFmpeg が必要です。後でこのセットアップを再実行すると取得できます。
english.FfPreparing=Finding the FFmpeg build…
korean.FfPreparing=FFmpeg 빌드를 찾는 중…
japanese.FfPreparing=FFmpeg のビルドを探しています…
english.FfUnpacking=Setting up FFmpeg…
korean.FfUnpacking=FFmpeg를 설치하는 중…
japanese.FfUnpacking=FFmpeg を設定しています…

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "ffmpeg"; Description: "{cm:FfTask}"; GroupDescription: "{cm:FfGroup}"; Check: not FfmpegFound

[InstallDelete]
; the PowerShell setup's own files and its shortcuts (named in its language)
Type: files; Name: "{app}\uninstall.ps1"
Type: files; Name: "{app}\msg.ps1"
Type: files; Name: "{group}\*.lnk"

[Files]
Source: "{#Src}\{#ExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Src}\fonts\*"; DestDir: "{app}\fonts"; Flags: ignoreversion
Source: "{#Src}\manual\*"; DestDir: "{app}\manual"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Src}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Src}\NOTICE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Src}\THIRD-PARTY-NOTICES.html"; DestDir: "{app}"; Flags: ignoreversion
Source: "ffmpeg.ps1"; Flags: dontcopy

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#ExeName}"
Name: "{group}\{cm:ManualLink}"; Filename: "{app}\manual\index.html"; IconFilename: "{app}\{#ExeName}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#ExeName}"; Tasks: desktopicon

[Registry]
; .void projects open in the app (this user)
Root: HKA; Subkey: "Software\Classes\.void"; ValueType: string; ValueName: ""; ValueData: "DriftingVOID.Project"; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\DriftingVOID.Project"; ValueType: string; ValueName: ""; ValueData: "{cm:FileType}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\DriftingVOID.Project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#ExeName}"",0"
Root: HKA; Subkey: "Software\Classes\DriftingVOID.Project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#ExeName}"" ""%1"""

[Run]
Filename: "{app}\{#ExeName}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
; an update from inside the app (/SILENT /relaunch=1): start it again
Filename: "{app}\{#ExeName}"; Flags: nowait; Check: Relaunch

[UninstallDelete]
; FFmpeg was fetched by this setup into the app's folder; the user's
; projects and settings (%APPDATA%\DriftingVOID) stay
Type: filesandordirs; Name: "{app}\ffmpeg"

[Code]
var
  DownloadPage: TDownloadWizardPage;
  FfZip, FfName, FfSum, FfError: String;

function Relaunch: Boolean;
begin
  Result := ExpandConstant('{param:relaunch|0}') = '1';
end;

function FfmpegFound: Boolean;
var
  App: String;
begin
  App := WizardDirValue;
  Result := FileExists(App + '\ffmpeg\bin\ffmpeg.exe')
    or FileExists(ExpandConstant('{userappdata}\DriftingVOID\ffmpeg\bin\ffmpeg.exe'))
    or (FileSearch('ffmpeg.exe', GetEnv('PATH')) <> '');
end;

{ ffmpeg.ps1, hidden; its answer is the first line of Out }
function RunHelper(const Args, OutFile: String; var Answer: String): Boolean;
var
  Code: Integer;
  Lines: TArrayOfString;
begin
  Answer := '';
  DeleteFile(OutFile);
  Result := Exec('powershell.exe', '-NoProfile -ExecutionPolicy Bypass -File "' + ExpandConstant('{tmp}\ffmpeg.ps1') + '" ' + Args + ' -Out "' + OutFile + '"',
    '', SW_HIDE, ewWaitUntilTerminated, Code);
  if LoadStringsFromFile(OutFile, Lines) and (GetArrayLength(Lines) > 0) then
    Answer := Lines[0];
  Result := Result and (Code = 0) and (Pos('error|', Answer) <> 1);
  if (Answer = '') and not Result then
    Answer := 'error|PowerShell ' + IntToStr(Code);
end;

procedure InitializeWizard;
begin
  DownloadPage := CreateDownloadPage(SetupMessage(msgWizardPreparing), SetupMessage(msgPreparingDesc), nil);
  DownloadPage.ShowBaseNameInsteadOfUrl := True;
end;

function NextButtonClick(CurPageID: Integer): Boolean;
var
  Answer, Url: String;
  P: Integer;
begin
  Result := True;
  FfZip := '';
  if (CurPageID <> wpReady) or not WizardIsTaskSelected('ffmpeg') then
    Exit;
  { pick the build and its checksum, then download it with progress; a
    failure leaves the app's install to go on without FFmpeg }
  ExtractTemporaryFile('ffmpeg.ps1');
  DownloadPage.SetText(CustomMessage('FfPreparing'), '');
  DownloadPage.Show;
  try
    if not RunHelper('-Resolve', ExpandConstant('{tmp}\ff-resolve.txt'), Answer) then begin
      FfError := Copy(Answer, 7, 400);
      Exit;
    end;
    P := Pos('|', Answer); Url := Copy(Answer, 1, P - 1); Delete(Answer, 1, P);
    P := Pos('|', Answer); FfSum := Copy(Answer, 1, P - 1); Delete(Answer, 1, P);
    P := Pos('|', Answer); FfName := Copy(Answer, 1, P - 1);
    DownloadPage.Clear;
    DownloadPage.Add(Url, FfName, FfSum);
    try
      DownloadPage.Download;
      FfZip := ExpandConstant('{tmp}\') + FfName;
    except
      if DownloadPage.AbortedByUser then
        FfError := SetupMessage(msgErrorDownloadAborted)
      else
        FfError := GetExceptionMessage;
    end;
  finally
    DownloadPage.Hide;
  end;
end;

procedure RemoveOldSetup;
begin
  { the PowerShell setup's Apps entry (this setup adds its own) }
  if RegKeyExists(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\DriftingVOID') then
    RegDeleteKeyIncludingSubkeys(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\DriftingVOID');
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Answer: String;
begin
  if CurStep = ssInstall then
    RemoveOldSetup;
  if CurStep <> ssPostInstall then
    Exit;
  if WizardIsTaskSelected('ffmpeg') then begin
    if FfZip <> '' then begin
      WizardForm.StatusLabel.Caption := CustomMessage('FfUnpacking');
      if not RunHelper('-Install -Zip "' + FfZip + '" -Name "' + FfName + '" -Sum ' + FfSum + ' -Dir "' + ExpandConstant('{app}\ffmpeg') + '"',
        ExpandConstant('{tmp}\ff-install.txt'), Answer) then
        FfError := Copy(Answer, 7, 400);
    end;
    if FfError <> '' then
      SuppressibleMsgBox(FmtMessage(CustomMessage('FfFailed'), [FfError]), mbInformation, MB_OK, IDOK);
  end;
end;
