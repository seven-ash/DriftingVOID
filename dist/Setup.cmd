@echo off
rem Drifting VOID beta setup (per user, no administrator rights)
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup\setup.ps1" %*
if errorlevel 1 pause
