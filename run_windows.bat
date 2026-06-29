@echo off
setlocal
cd /d "%~dp0"

if exist "dist\RunScope.exe" (
  "dist\RunScope.exe"
  exit /b %ERRORLEVEL%
)

if exist "target\release\runscope.exe" (
  "target\release\runscope.exe"
  exit /b %ERRORLEVEL%
)

cargo run --release
endlocal
