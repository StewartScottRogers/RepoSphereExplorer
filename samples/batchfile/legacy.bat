@echo off
setlocal

rem An older build script, kept for the machines that still need it.
rem
rem `:cleanup` was deleted when the temporary directory stopped being
rem used, and the jump to it was not. Running this script far enough
rem stops it dead with "The system cannot find the batch label
rem specified - cleanup". It is left that way so the file pane has
rem something to warn about.

set "ROOT=%~dp0"
set "OUTPUT=%ROOT%out"

if not exist "%OUTPUT%" mkdir "%OUTPUT%"

cargo build --release
if errorlevel 1 goto :cleanup

xcopy /y "%ROOT%target\release\*.exe" "%OUTPUT%\"
if errorlevel 1 goto :cleanup

echo Built into %OUTPUT%
endlocal
exit /b 0
