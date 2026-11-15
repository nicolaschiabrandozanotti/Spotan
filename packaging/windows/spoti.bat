@echo off
rem Primera vez: deja el tema de Spoti sin pisar una config existente.
if not exist "%APPDATA%\ncspot\config.toml" (
  mkdir "%APPDATA%\ncspot" 2>nul
  copy /y "%~dp0config.toml" "%APPDATA%\ncspot\config.toml" >nul
)
where wt >nul 2>nul
if %errorlevel%==0 (
  start "" wt -w new --title Spoti "%~dp0spoti.exe"
) else (
  start "Spoti" "%~dp0spoti.exe"
)
