@echo off
setlocal
set "PUPPET_MASTER_PROFILE=potato"
cd /d "%~dp0"
npm run dev
