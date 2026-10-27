@echo off
setlocal
cd /d "%~dp0"

echo [1/3] Installing dependencies...
python -m pip install --quiet --upgrade -r requirements-dev.txt
if errorlevel 1 goto error

echo [2/3] Generating icon...
python make_icon.py
if errorlevel 1 goto error

echo [3/3] Building exe...
python -m PyInstaller --noconfirm --clean --onefile --windowed ^
    --name LamzuBattery --icon icon.ico ^
    --exclude-module tkinter --exclude-module numpy app.py
if errorlevel 1 goto error

echo.
echo Done: %CD%\dist\LamzuBattery.exe
pause
exit /b 0

:error
echo.
echo Build failed.
pause
exit /b 1
