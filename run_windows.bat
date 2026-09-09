@echo off
setlocal
cd /d "%~dp0"
if exist "dist\RunScope.exe" goto dist
if exist "target\release\runscope.exe" goto release
cargo run --release -- %*
exit /b %ERRORLEVEL%
:dist
"dist\RunScope.exe" %*
exit /b %ERRORLEVEL%
:release
"target\release\runscope.exe" %*
exit /b %ERRORLEVEL%
