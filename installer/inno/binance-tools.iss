[Setup]
AppName=Binance Tools
AppVersion=0.1.0
DefaultDirName={localappdata}\Binance Tools
DefaultGroupName=Binance Tools
OutputDir=..\..\target\installer
OutputBaseFilename=BinanceToolsSetup
SetupIconFile=..\..\examples\desktop-gpui\assets\app.ico
Compression=lzma
SolidCompression=yes
DisableDirPage=no
DisableProgramGroupPage=no

[Files]
Source: "..\..\target\release\desktop-gpui.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Binance Tools"; Filename: "{app}\desktop-gpui.exe"; WorkingDir: "{app}"
Name: "{commondesktop}\Binance Tools"; Filename: "{app}\desktop-gpui.exe"; WorkingDir: "{app}"

[Run]
Filename: "{app}\desktop-gpui.exe"; WorkingDir: "{app}"; Description: "Launch Binance Tools"; Flags: nowait postinstall skipifsilent
