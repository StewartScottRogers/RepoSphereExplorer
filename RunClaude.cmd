@echo off
setlocal
pushd "%~dp0"
REM Use the latest, most capable model by default; honour CLAUDE_MODEL if the caller set one
REM (e.g. hrdrClaudeNative.cmd forwards it into the herdr pane).
if not defined CLAUDE_MODEL set "CLAUDE_MODEL=claude-opus-4-8"
call claude --model %CLAUDE_MODEL% --dangerously-skip-permissions --verbose
popd
endlocal
