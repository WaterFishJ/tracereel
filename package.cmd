@echo off
setlocal
title TraceReel - Build portable package
cd /d "%~dp0"
echo Building TraceReel and creating a portable ZIP...
echo.
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0package.ps1" %*
set "package_exit=%errorlevel%"
echo.
if not "%package_exit%"=="0" (
    echo Packaging failed. See the error above.
) else (
    echo Packaging complete. Send the newest portable.zip in the dist folder.
)
if not defined TRACEREEL_PACKAGE_NO_PAUSE pause
exit /b %package_exit%
