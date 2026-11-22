@echo off
rem Earth Online: walk and drive through San Francisco.
rem Double-click to start. From a command prompt, extra switches pass through:
rem   play.bat --beaver-house
cd /d "%~dp0"
eo-app.exe --play %*
if errorlevel 1 (
  echo.
  echo Earth Online stopped with an error. The lines above say why; PLAYING.md lists common fixes.
  pause
)
