@echo off
setlocal EnableDelayedExpansion

REM  Anchor to this script's own folder so every step runs relative to where the
REM  .cmd lives, not the caller's current directory (%~dp0 is that folder, with a
REM  trailing backslash). This makes the launched herdr server and all path
REM  detection deterministic no matter where the script is invoked from.
cd /d "%~dp0"

REM =============================================================================
REM  hrdrClaudeNative.cmd  --  one-file portable Claude-in-herdr dev bootstrap
REM =============================================================================
REM
REM  WHAT THIS IS
REM  ------------
REM  A single, self-contained batch file. Email it to yourself, run it on a bare
REM  Windows machine, and in one execution it stands up a working dev environment
REM  for the RepoSphereExplorer project -- no other files required.
REM
REM  It performs these steps, each a no-op if already satisfied:
REM
REM      [1/7] Ensure git            (installs via winget if missing)
REM      [2/7] Ensure Node.js / npm  (installs via winget if missing)
REM      [3/7] Ensure Claude Code    (installs/updates via npm)
REM      [4/7] Ensure herdr          (installs/updates the terminal multiplexer)
REM      [5/7] Clone / update repo   (into %USERPROFILE%\RepoSphereExplorer)
REM      [6/7] Ensure herdr server   (starts a compatible one if needed)
REM      [7/7] Launch Claude         (as a tracked pane anchored to the repo)
REM
REM  Claude shows up as a tracked "Claude" pane in herdr's sidebar; its live state
REM  (idle / working / blocked) is reported by herdr's native Claude integration
REM  hook. Step [4/7] guarantees both the "claude" and "copilot" integration hooks
REM  are installed, running each install unless status confirms it already is.
REM
REM  WHERE IT RUNS THE REPO FROM
REM  ---------------------------
REM  * Launched from inside an existing checkout  -> uses that checkout as-is.
REM  * Launched from anywhere else (e.g. Downloads) -> clones a fresh copy into
REM    %USERPROFILE%\RepoSphereExplorer.
REM
REM  REQUIREMENTS ON A BARE MACHINE
REM  ------------------------------
REM  * winget (ships with Windows 10 1809+ / Windows 11) is used to install git
REM    and Node.js when they are absent; those MSI installs may raise a UAC prompt.
REM  * The repo is public, so cloning needs no credentials.
REM  * Claude Code will ask you to sign in on first run -- that cannot be baked in.
REM
REM  CUSTOMISING THE MODEL
REM  ---------------------
REM      setx CLAUDE_MODEL "claude-sonnet-5"
REM  (default below is claude-opus-5, the latest and most capable model)
REM
REM  IMPLEMENTATION NOTE
REM  -------------------
REM  Batch has no exceptions, so every fatal condition sets ERRMSG and jumps to the
REM  ":die" handler at the bottom, which prints the message, keeps the window open
REM  (so a double-click user can read it), and exits with code 1.
REM =============================================================================


REM -----------------------------------------------------------------------------
REM  Configuration
REM -----------------------------------------------------------------------------
if not defined CLAUDE_MODEL set "CLAUDE_MODEL=claude-opus-5"
set "REPO_URL=https://github.com/StewartScottRogers/RepoSphereExplorer.git"
set "DEFAULT_REPO_DIR=%USERPROFILE%\RepoSphereExplorer"

REM  Decide which checkout to use. %~dp0 is the folder this script lives in (with a
REM  trailing backslash, which we strip so the path quotes cleanly later). If that
REM  folder is itself a checkout of this repo we use it in place; otherwise we fall
REM  back to the default clone location and clone into it in step 5.
set "SELF_DIR=%~dp0"
if "%SELF_DIR:~-1%"=="\" set "SELF_DIR=%SELF_DIR:~0,-1%"
set "REPO_DIR=%DEFAULT_REPO_DIR%"
if exist "%SELF_DIR%\.git" if exist "%SELF_DIR%\Cargo.toml" set "REPO_DIR=%SELF_DIR%"

REM  winget is our fallback installer for git and Node.js. Locate it once; a blank
REM  WINGET means "not available", which is only fatal if a tool actually needs it.
set "WINGET="
where winget >nul 2>nul && set "WINGET=winget"


REM -----------------------------------------------------------------------------
REM  [1/7]  Ensure git  (needed to clone the repo)
REM -----------------------------------------------------------------------------
REM  Prefer git on PATH; otherwise try its default install location. Only if both
REM  miss do we install it, then look again (a fresh winget install does not update
REM  THIS session's PATH, so we must probe the known path directly).
echo [1/7] Checking for git ...
set "GIT="
where git >nul 2>nul && set "GIT=git"
if not defined GIT if exist "%ProgramFiles%\Git\cmd\git.exe" set "GIT=%ProgramFiles%\Git\cmd\git.exe"
if defined GIT goto :git_ready

echo       git not found - installing via winget ...
if not defined WINGET (
    set "ERRMSG=winget is unavailable, so git cannot be auto-installed. Install Git for Windows from git-scm.com, then retry."
    goto :die
)
winget install -e --id Git.Git --silent --accept-package-agreements --accept-source-agreements
REM  A fresh winget install does not refresh THIS session's PATH, so extend it to
REM  git's install dir and then locate git (by PATH lookup, or the known path).
set "PATH=%PATH%;%ProgramFiles%\Git\cmd"
where git >nul 2>nul && set "GIT=git"
if not defined GIT if exist "%ProgramFiles%\Git\cmd\git.exe" set "GIT=%ProgramFiles%\Git\cmd\git.exe"

:git_ready
if not defined GIT (
    set "ERRMSG=git could not be installed or located. Install Git for Windows from git-scm.com, then retry."
    goto :die
)


REM -----------------------------------------------------------------------------
REM  [2/7]  Ensure Node.js / npm  (needed to install Claude Code)
REM -----------------------------------------------------------------------------
echo [2/7] Checking for Node.js / npm ...
set "NPM="
where npm >nul 2>nul && set "NPM=npm"
if not defined NPM if exist "%ProgramFiles%\nodejs\npm.cmd" set "NPM=%ProgramFiles%\nodejs\npm.cmd"
if defined NPM goto :node_ready

echo       Node.js not found - installing via winget ...
if not defined WINGET (
    set "ERRMSG=winget is unavailable, so Node.js cannot be auto-installed. Install Node.js LTS from nodejs.org, then retry."
    goto :die
)
winget install -e --id OpenJS.NodeJS.LTS --silent --accept-package-agreements --accept-source-agreements
REM  Extend PATH to Node's install dir (winget will not refresh it this session),
REM  then locate npm (by PATH lookup, or the known path).
set "PATH=%PATH%;%ProgramFiles%\nodejs"
where npm >nul 2>nul && set "NPM=npm"
if not defined NPM if exist "%ProgramFiles%\nodejs\npm.cmd" set "NPM=%ProgramFiles%\nodejs\npm.cmd"

:node_ready
if not defined NPM (
    set "ERRMSG=npm could not be installed or located. Install Node.js LTS from nodejs.org, then retry."
    goto :die
)

REM  Put Node and the npm global-bin directory on THIS session's PATH. This matters
REM  twice over: `claude` resolves here, and it also resolves inside the herdr
REM  server (and its panes) that we launch later, because a freshly started server
REM  inherits this console's environment.
set "PATH=%PATH%;%ProgramFiles%\nodejs;%APPDATA%\npm"


REM -----------------------------------------------------------------------------
REM  [3/7]  Ensure Claude Code  (npm global package), then update it
REM -----------------------------------------------------------------------------
REM  Missing  -> install. Present -> best-effort update (an offline failure is
REM  swallowed so the launch still proceeds on the version already installed).
echo [3/7] Checking for Claude Code ...
where claude >nul 2>nul
if not errorlevel 1 goto :claude_update

echo       installing Claude Code via npm ...
call "%NPM%" install -g @anthropic-ai/claude-code
goto :claude_ready

:claude_update
echo       updating Claude Code ...
call claude update 2>nul

:claude_ready


REM -----------------------------------------------------------------------------
REM  [4/7]  Ensure herdr, update it, and install its Claude integration hook
REM -----------------------------------------------------------------------------
REM  Locate herdr on PATH first, then at its default install location. If missing,
REM  run the official Windows installer (a PowerShell script that drops versioned
REM  binaries under %USERPROFILE%\.herdr and points %LOCALAPPDATA%\Programs\Herdr\
REM  bin at the current one), then probe the known path again.
echo [4/7] Checking for herdr ...
set "HERDR="
where herdr >nul 2>nul && set "HERDR=herdr"
if not defined HERDR if exist "%LOCALAPPDATA%\Programs\Herdr\bin\herdr.exe" set "HERDR=%LOCALAPPDATA%\Programs\Herdr\bin\herdr.exe"
if defined HERDR goto :herdr_ready

echo       herdr not found - installing ...
powershell -ExecutionPolicy Bypass -NoProfile -Command "irm https://herdr.dev/install.ps1 | iex"
where herdr >nul 2>nul && set "HERDR=herdr"
if not defined HERDR if exist "%LOCALAPPDATA%\Programs\Herdr\bin\herdr.exe" set "HERDR=%LOCALAPPDATA%\Programs\Herdr\bin\herdr.exe"

:herdr_ready
if not defined HERDR (
    set "ERRMSG=herdr could not be installed or located. Install it from herdr.dev, or add its bin directory to PATH, then retry."
    goto :die
)

REM  Keep herdr current (best-effort; ignore failures on an offline/pinned box).
echo       updating herdr ...
"%HERDR%" update 2>nul

REM  Guarantee both native integration hooks are installed -- "claude" so the pane
REM  reports Claude's state in the sidebar, and "copilot" alongside it. Each is
REM  ensured idempotently: we install unless `integration status` EXPLICITLY reports
REM  that hook as already installed, so any uncertainty (unreadable status, changed
REM  wording) errs on the side of running the install rather than skipping it. An
REM  already-installed hook that status confirms is left untouched (no needless churn).
call :ensure_integration claude
call :ensure_integration copilot


REM -----------------------------------------------------------------------------
REM  [5/7]  Clone or update the repo  (skipped when using an in-place checkout)
REM -----------------------------------------------------------------------------
REM  We only touch the DEFAULT clone location. If REPO_DIR points at a checkout the
REM  script was launched from, we leave that working tree strictly alone.
echo [5/7] Preparing the repository ...
if /I not "%REPO_DIR%"=="%DEFAULT_REPO_DIR%" goto :repo_ready

if exist "%REPO_DIR%\.git" (
    echo       updating existing checkout ...
    "%GIT%" -C "%REPO_DIR%" pull --ff-only
) else (
    echo       cloning into "%REPO_DIR%" ...
    "%GIT%" clone "%REPO_URL%" "%REPO_DIR%"
    if errorlevel 1 (
        set "ERRMSG=could not clone %REPO_URL%. Check your network connection and retry."
        goto :die
    )
)

:repo_ready
if not exist "%REPO_DIR%\.git" (
    set "ERRMSG=no repo checkout found at %REPO_DIR%."
    goto :die
)


REM -----------------------------------------------------------------------------
REM  [6/7]  Ensure a COMPATIBLE herdr server is running to host the pane
REM -----------------------------------------------------------------------------
REM  A server from an older herdr build still answers "status: running" but also
REM  "compatible: no" (its wire protocol is older than this client's). Every
REM  pane/agent API call against such a server fails with protocol_mismatch, so we
REM  treat "running but not compatible" as not-ready: stop it here, then fall
REM  through to start a fresh one on the current build.
echo [6/7] Ensuring a herdr server is running ...
set "SRV_RUNNING="
set "SRV_COMPAT="
"%HERDR%" status server 2>nul | findstr /I /C:"status: running" >nul && set "SRV_RUNNING=1"
"%HERDR%" status server 2>nul | findstr /I /C:"compatible: yes" >nul && set "SRV_COMPAT=1"
if defined SRV_RUNNING if defined SRV_COMPAT goto :server_ready

if defined SRV_RUNNING if not defined SRV_COMPAT (
    echo       restarting outdated herdr server ^(protocol mismatch^) ...
    "%HERDR%" server stop >nul 2>nul
)

echo       starting a herdr session ...
start "herdr" "%HERDR%"
set "STARTED_HERDR=1"

REM  Wait (up to ~20s) for the server to accept API calls. "Ready" means both the
REM  server reports running AND `pane list` returns a pane, i.e. the session is
REM  fully up rather than merely mid-boot.
for /l %%i in (1,1,20) do (
    "%HERDR%" status server 2>nul | findstr /I /C:"status: running" >nul
    if not errorlevel 1 (
        "%HERDR%" pane list 2>nul | findstr /I /C:"pane_id" >nul
        if not errorlevel 1 goto :server_ready
    )
    timeout /t 1 /nobreak >nul 2>&1
)
set "ERRMSG=herdr did not become ready in time."
goto :die


REM -----------------------------------------------------------------------------
REM  [7/7]  Launch Claude as a tracked pane inside herdr
REM -----------------------------------------------------------------------------
:server_ready
echo [7/7] Launching Claude in herdr ...

REM  Choose a friendly, incrementing tab label ("Claude", "Claude 2", ...) by
REM  counting the claude agents herdr already tracks. The agent itself is
REM  identified by detection (herdr's integration hook), not by this label, so the
REM  label is display-only. PowerShell parses herdr's JSON API output for the count.
set "CLAUDE_COUNT=0"
for /f "usebackq delims=" %%N in (`powershell -NoProfile -Command "@((& '%HERDR%' agent list | ConvertFrom-Json).result.agents | Where-Object { $_.agent -eq 'claude' }).Count"`) do set "CLAUDE_COUNT=%%N"
set /a LABEL_N=CLAUDE_COUNT+1
REM  Put the containing folder's name in the tab caption so Claude tabs opened on
REM  different checkouts/folders are easy to tell apart. %%~nxI is the last path
REM  component of REPO_DIR (the tab's --cwd), i.e. the folder name.
for %%I in ("%REPO_DIR%") do set "REPO_NAME=%%~nxI"
if "%LABEL_N%"=="1" (set "AGENT_LABEL=Claude - %REPO_NAME%") else (set "AGENT_LABEL=Claude %LABEL_N% - %REPO_NAME%")

REM  Create the pane that will host Claude. Newer herdr (protocol 17+) redesigned
REM  the launch flow: `agent start` now attaches to an EXISTING pane by id, and the
REM  cwd/env options moved onto pane/tab creation. So we create a labelled tab
REM  anchored to the repo (with CLAUDE_MODEL forwarded into its environment) and
REM  read the new pane's id out of the JSON response.
set "PANE_ID="
for /f "usebackq delims=" %%P in (`powershell -NoProfile -Command "(& '%HERDR%' tab create --cwd '%REPO_DIR%' --env CLAUDE_MODEL=%CLAUDE_MODEL% --label '%AGENT_LABEL%' --focus | ConvertFrom-Json).result.root_pane.pane_id"`) do set "PANE_ID=%%P"
if not defined PANE_ID (
    set "ERRMSG=herdr could not create a pane for Claude."
    goto :die
)

REM  Start Claude by TYPING its command into the pane's shell. We deliberately do
REM  not use `agent start --kind claude`: on Windows that spawns the agent via
REM  PowerShell's Start-Process, which cannot execute the npm `claude.cmd` shim
REM  ("%1 is not a valid Win32 application"). Typing the command runs Claude through
REM  the shell exactly as a human would, and herdr's integration hook then detects
REM  it and tracks the pane. These are the flags the old RunClaude.cmd used, now
REM  inlined so no companion file is needed.
REM
REM  wait-output first blocks until the pane's shell prints a prompt (a ">"), so we
REM  do not fire keystrokes into a shell that has not finished starting.
"%HERDR%" pane wait-output "%PANE_ID%" --match ">" --timeout 15000 >nul 2>nul
"%HERDR%" pane send-text "%PANE_ID%" "claude --model %CLAUDE_MODEL% --dangerously-skip-permissions --verbose"
"%HERDR%" pane send-keys "%PANE_ID%" Enter
if errorlevel 1 (
    set "ERRMSG=herdr could not launch Claude in the pane."
    goto :die
)


REM -----------------------------------------------------------------------------
REM  Done -- attach this console to the herd, unless we opened a new window for it
REM -----------------------------------------------------------------------------
REM  If we started the server ourselves it came up in its own new window, so there
REM  is nothing to attach here -- just point the user at that window and exit. If a
REM  server was already running, we attach this console to it so Claude is right in
REM  front of the user (detach later with the herdr prefix, Ctrl+B then d).
if defined STARTED_HERDR (
    echo.
    echo %AGENT_LABEL% is running in the new herdr window - switch to it.
    endlocal
    exit /b 0
)
echo.
echo Attaching to the herd ^(detach with the herdr prefix, Ctrl+B then d^) ...
"%HERDR%"
endlocal
exit /b 0


REM -----------------------------------------------------------------------------
REM  Fatal-error handler (reached only via "goto :die" with ERRMSG set)
REM -----------------------------------------------------------------------------
:die
echo.
echo ERROR: %ERRMSG%
echo.
pause
endlocal
exit /b 1


REM -----------------------------------------------------------------------------
REM  :ensure_integration <name>  --  guarantee `herdr integration install <name>`
REM  has run. Skips the install ONLY when `integration status` explicitly reports
REM  "<name>: installed"; every other outcome (not installed, unknown, unreadable
REM  status) falls through to the install, which herdr treats as idempotent. The
REM  findstr regex matches "<name>:" + optional spaces + "installed", so the
REM  "<name>: not installed" line does NOT satisfy it (the "not" breaks the match).
REM -----------------------------------------------------------------------------
:ensure_integration
"%HERDR%" integration status 2>nul | findstr /I /R /C:"%~1: *installed" >nul
if not errorlevel 1 (
    echo       herdr %~1 integration already installed
    goto :eof
)
echo       installing herdr %~1 integration hook ...
"%HERDR%" integration install %~1
goto :eof
