@echo off
chcp 65001 >nul 2>&1
setlocal EnableExtensions
title lolGang DEV (instance 2)
cd /d "%~dp0"

rem ---------------------------------------------------------------
rem  lolGang one-click launcher (Windows)
rem
rem  Double click          -> start Tauri dev (the app opens)
rem  start.bat stop        -> stop a running dev session
rem  start.bat --check     -> inspect environment only, launch nothing
rem  start.bat build       -> frontend production build only
rem  start.bat release     -> build the NSIS installer
rem
rem  No project config is modified. A temporary pnpm shim is created
rem  under %%TEMP%% so tauri.conf.json "pnpm dev" resolves correctly.
rem
rem  Runs as instance 2. The installed release (lolPing) and this source
rem  share identifier com.lijia.lolgang, so the single-instance plugin
rem  would forward our launch to the installed app and this build would
rem  exit immediately. Instance 2 skips that plugin and uses its own
rem  data dir, keystore and UDP port. It also skips update checks, which
rem  would otherwise try to reinstall the release over the dev build.
rem ---------------------------------------------------------------

set "COREPACK_ENABLE_DOWNLOAD_PROMPT=0"
set "SHIM=%TEMP%\lolgang-launch"
set "MODE=%~1"
set "CHECK="
if not defined LOLGANG_INSTANCE set "LOLGANG_INSTANCE=2"
if /i "%MODE%"=="--check" set "CHECK=1"

echo.
echo   lolGang  /  one-click launcher
echo   -----------------------------------------------
echo.

call :ENSURE_NODE
if errorlevel 1 goto :NO_NODE
echo   [1/4] Node.js     OK

call :ENSURE_CARGO
if errorlevel 1 goto :NO_CARGO
echo   [2/4] Rust cargo  OK

call :ENSURE_PNPM
if errorlevel 1 goto :NO_PNPM
echo   [3/4] pnpm        OK

if defined CHECK goto :CHECK_DONE

if /i "%MODE%"=="stop" goto :STOP_MODE

if exist "node_modules" goto :DEPS_OK
echo   [4/4] Installing dependencies, first run only ...
call %PNPM% install --frozen-lockfile
if errorlevel 1 goto :FAIL
:DEPS_OK
echo   [4/4] Dependencies OK
echo.

if /i "%MODE%"=="build" (
  echo   Mode: frontend build
  call %PNPM% build
  if errorlevel 1 goto :FAIL
  goto :DONE
)

if /i "%MODE%"=="release" (
  echo   Mode: build installer, this takes a while ...
  call %PNPM% release
  if errorlevel 1 goto :FAIL
  goto :DONE
)

rem ---------------------------------------------------------------
rem  A session closed with the X button can leave Vite running and
rem  holding port 1420. tauri.conf.json pins devUrl to that port, so the
rem  next launch dies with "Port 1420 is already in use" before the
rem  window ever opens. Clear it up front instead of failing.
rem ---------------------------------------------------------------
echo   Checking port 1420 ...
call :KILL_APP
call :KILL_PORT
if errorlevel 1 goto :FAIL_HINT

echo   Mode: dev  [LOLGANG_INSTANCE=%LOLGANG_INSTANCE%]
echo   This is the PROJECT build, not the installed lolPing.
echo   Separate data dir / identity / UDP port - the installed app is
echo   left running and untouched.
echo   First run can take 5-10 minutes. Press Ctrl+C or close this window to stop.
echo.
call %PNPM% tauri dev
set "DEV_CODE=%errorlevel%"
if not "%DEV_CODE%"=="0" goto :DEV_STOP
goto :DONE

rem ---------------------------------------------------------------
:STOP_MODE
echo   Stopping the dev session ...
call :KILL_APP
call :KILL_PORT
if errorlevel 1 goto :FAIL_HINT
echo   Port 1420 is free.
goto :DONE

rem ---------------------------------------------------------------
:ENSURE_NODE
where node >nul 2>&1
if not errorlevel 1 exit /b 0
set "NB=%USERPROFILE%\.workbuddy\binaries\node\versions"
if exist "%NB%" (
  for /f "delims=" %%d in ('dir /b /ad /o-n "%NB%" 2^>nul') do (
    if exist "%NB%\%%d\node.exe" (
      set "PATH=%NB%\%%d;%PATH%"
      exit /b 0
    )
  )
)
if exist "C:\Program Files\nodejs\node.exe" set "PATH=C:\Program Files\nodejs;%PATH%"
where node >nul 2>&1
exit /b %errorlevel%

:ENSURE_CARGO
where cargo >nul 2>&1
if not errorlevel 1 exit /b 0
if exist "%USERPROFILE%\.cargo\bin" set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
where cargo >nul 2>&1
exit /b %errorlevel%

:ENSURE_PNPM
where pnpm >nul 2>&1
if not errorlevel 1 (
  set "PNPM=pnpm"
  pnpm --version >nul 2>&1
  exit /b %errorlevel%
)
if not exist "%SHIM%" mkdir "%SHIM%" >nul 2>&1
call corepack enable --install-directory "%SHIM%" >nul 2>&1
if errorlevel 1 exit /b 1
set "PATH=%SHIM%;%PATH%"
set "PNPM=corepack pnpm"
call corepack pnpm --version >nul 2>&1
exit /b %errorlevel%

rem ---------------------------------------------------------------
rem  Close the dev app (target\debug\lolgang.exe). The installed release
rem  is lolping.exe, so it is never touched by this.
:KILL_APP
taskkill /IM lolgang.exe /F >nul 2>&1
exit /b 0

rem  End the process listening on 1420, but only when it is a node dev
rem  server (port 1420 is this project's pinned devUrl, so a node there
rem  is a leftover from a previous session). Anything else is reported
rem  and never killed. exit /b 1 means the port is still not usable.
:KILL_PORT
set "PORT_PID="
for /f "tokens=5" %%p in ('netstat -ano ^| findstr /r /c:":1420[^0-9]" ^| findstr /c:"LISTENING"') do set "PORT_PID=%%p"
if not defined PORT_PID exit /b 0
set "PORT_IMG="
for /f "tokens=1 delims=," %%n in ('tasklist /FI "PID eq %PORT_PID%" /FO CSV /NH 2^>nul') do set "PORT_IMG=%%~n"
if not defined PORT_IMG exit /b 0
if /i not "%PORT_IMG%"=="node.exe" goto :KILL_PORT_FOREIGN
echo         stale dev server found (%PORT_IMG%, PID %PORT_PID%), closing it ...
taskkill /PID %PORT_PID% /T /F >nul 2>&1
set /a PORT_WAIT=0
:KILL_PORT_LOOP
set "PORT_STILL="
for /f "tokens=5" %%p in ('netstat -ano ^| findstr /r /c:":1420[^0-9]" ^| findstr /c:"LISTENING"') do set "PORT_STILL=1"
if not defined PORT_STILL (
  echo         port 1420 free
  exit /b 0
)
set /a PORT_WAIT+=1
if %PORT_WAIT% GEQ 8 exit /b 1
ping -n 2 127.0.0.1 >nul
goto :KILL_PORT_LOOP

:KILL_PORT_FOREIGN
echo.
echo   [X] Port 1420 is in use by another program, so the dev server
echo       cannot start.
echo       PID   %PORT_PID%
echo       Image %PORT_IMG%
echo       Close that program and run this script again.
exit /b 1

:PORT_STATUS
set "PORT_PID="
for /f "tokens=5" %%p in ('netstat -ano ^| findstr /r /c:":1420[^0-9]" ^| findstr /c:"LISTENING"') do set "PORT_PID=%%p"
if not defined PORT_PID (
  echo         port 1420 free
) else (
  set "PORT_IMG="
  for /f "tokens=1 delims=," %%n in ('tasklist /FI "PID eq %PORT_PID%" /FO CSV /NH 2^>nul') do set "PORT_IMG=%%~n"
  echo         port 1420 IN USE by %PORT_IMG% (PID %PORT_PID%) - run  start.bat stop
)
exit /b 0

rem ---------------------------------------------------------------
:CHECK_DONE
echo   [4/4] Dependencies ...
if exist "node_modules" (
  echo         node_modules present
) else (
  echo         node_modules missing - will be installed on first launch
)
call :PORT_STATUS
echo.
for /f "delims=" %%v in ('node -v') do echo   node    %%v
for /f "delims=" %%v in ('cargo -V') do echo   cargo   %%v
call %PNPM% --version
echo   Dev instance %LOLGANG_INSTANCE% - isolated from the installed release.
echo.
echo   Environment OK. Double-click start.bat to launch the app.
echo.
exit /b 0

:NO_NODE
echo   [X] Node.js not found. Install Node.js 20+ from https://nodejs.org
goto :FAIL_HINT

:NO_CARGO
echo   [X] Rust cargo not found. Install from https://rustup.rs
goto :FAIL_HINT

:NO_PNPM
echo   [X] pnpm / corepack not available. Reinstall Node.js 20+ with Corepack.
goto :FAIL_HINT

:FAIL
echo.
echo   [X] Command failed. See the message above.
goto :FAIL_HINT

:FAIL_HINT
echo.
echo   Tip: run  start.bat --check  to inspect the environment.
echo.
pause
exit /b 1

rem  Closing the app or pressing Ctrl+C makes tauri dev exit non-zero.
rem  That is a normal stop, not a failure - do not show an error.
:DEV_STOP
echo.
echo   Dev session ended. Closing the app window or pressing Ctrl+C is normal.
echo   If Rust or Vite printed an error above, run  start.bat --check.
echo   If port 1420 is still held after a crash, run  start.bat stop.
echo.
pause
exit /b 0

:DONE
echo.
echo   Done.
echo.
pause
exit /b 0
