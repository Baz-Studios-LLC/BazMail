@echo off
REM Same as run.sh, for double-clicking or running from PowerShell/cmd.
cd /d "%~dp0app"

REM Closing the app window can leave Vite holding the dev port; clear it first.
for /f "tokens=5" %%p in ('netstat -ano ^| findstr ":1420" ^| findstr "LISTENING"') do (
  echo Clearing stale listener on port 1420 ^(pid %%p^)...
  taskkill /PID %%p /F >nul 2>&1
)

if not exist node_modules (
  echo Installing frontend dependencies...
  call npm install
)
call npm run tauri dev
