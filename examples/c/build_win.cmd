@echo off
rem struct_docparse C example build (Windows, MSVC x64)
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
cd /d "%~dp0"
cl /nologo /utf-8 /I ..\..\include example.c ..\..\target\release\struct_docparse.dll.lib shell32.lib /Fe:example.exe
