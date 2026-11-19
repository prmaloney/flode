; Inno Setup script for the Windows installer. CI runs it after `xtask bundle flode --release`:
;   ISCC /DAppVersion=0.1.0 /DLabel=0.1.0 assets\windows\installer.iss
; AppVersion is the version shown in the installer. Label goes into the file name, and is the same
; as the version except for nightly builds. The bundles come from target\bundled.

#ifndef AppVersion
  #error Pass /DAppVersion=<version>
#endif
#ifndef Label
  #define Label AppVersion
#endif

[Setup]
AppId={{B6C3F0A2-5D0B-4B8E-9B35-7C1B0F4C2E61}
AppName=Flode
AppVersion={#AppVersion}
AppPublisher=Patrick Maloney
AppPublisherURL=https://github.com/prmaloney/flode
DefaultDirName={autopf}\Flode
DefaultGroupName=Flode
LicenseFile=..\..\LICENSE
OutputDir=..\..\dist
OutputBaseFilename=Flode-{#Label}-Windows-Setup
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayName=Flode

[Components]
Name: "vst3"; Description: "VST3 plugin"; Types: full compact custom
Name: "clap"; Description: "CLAP plugin"; Types: full custom
Name: "standalone"; Description: "Standalone app"; Types: full custom

[Types]
Name: "full"; Description: "All formats and the standalone app"
Name: "compact"; Description: "VST3 plugin only"
Name: "custom"; Description: "Choose what to install"; Flags: iscustom

[Files]
; flode.vst3 is a bundle folder here, which hosts look for in the common VST3 folder.
Source: "..\..\target\bundled\flode.vst3\*"; DestDir: "{commoncf64}\VST3\flode.vst3"; Components: vst3; Flags: ignoreversion recursesubdirs createallsubdirs
; flode.clap is a single file.
Source: "..\..\target\bundled\flode.clap"; DestDir: "{commoncf64}\CLAP"; Components: clap; Flags: ignoreversion
Source: "..\..\target\bundled\flode.exe"; DestDir: "{app}"; Components: standalone; Flags: ignoreversion

[Icons]
Name: "{group}\Flode"; Filename: "{app}\flode.exe"; Components: standalone
