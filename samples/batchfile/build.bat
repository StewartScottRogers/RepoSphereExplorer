@echo off
setlocal enabledelayedexpansion

rem Build, test and package the Repos Explorer.
::
:: `:package` is never called and nothing jumps to it, so it runs only
:: if `:test` falls through into it. It is left that way so the file
:: pane has something to warn about.

set "ROOT=%~dp0"
set "TARGET=%ROOT%target\release"
set /a FAILURES=0

if "%~1"=="" (
    set "PROFILE=release"
) else (
    set "PROFILE=%~1"
)

echo Building %PROFILE% from %ROOT%

call :build
if errorlevel 1 goto :failed

call :test
if errorlevel 1 goto :failed

goto :done

:build
cargo build --profile %PROFILE%
exit /b %errorlevel%

:test
cargo fmt --all --check
if errorlevel 1 exit /b 1
cargo clippy --all-targets --all-features -- -D warnings
if errorlevel 1 exit /b 1
cargo test --all-features
exit /b %errorlevel%

:package
powershell -NoProfile -Command "Compress-Archive -Path '%TARGET%\*' -DestinationPath '%ROOT%explorer.zip' -Force"
exit /b %errorlevel%

:failed
set /a FAILURES=!FAILURES! + 1
echo Build failed with !FAILURES! failure^(s^)
endlocal
exit /b 1

:done
echo Done.
endlocal
exit /b 0
