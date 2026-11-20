@echo off
setlocal
cd /d "%~dp0"

set "CARGO=%USERPROFILE%\.cargo\bin\cargo.exe"
if not exist "%CARGO%" (
    echo Rust is not installed. Install it from https://rustup.rs and try again.
    exit /b 1
)

echo [1/5] Formatting check...
"%CARGO%" fmt --check || goto error

echo [2/5] Lints...
"%CARGO%" clippy --all-targets -- -D warnings || goto error

echo [3/5] Tests...
"%CARGO%" test || goto error

echo [4/5] Release build...
"%CARGO%" build --release --locked || goto error

echo [5/5] Packaging...
if not exist dist mkdir dist
copy /y target\release\LamzuBattery.exe dist\LamzuBattery.exe >nul || goto error
certutil -hashfile dist\LamzuBattery.exe SHA256 > dist\SHA256SUMS.txt

echo.
echo Done: %CD%\dist\LamzuBattery.exe
exit /b 0

:error
echo.
echo Build failed.
exit /b 1
