#!/usr/bin/env bash
# Local recipe for a future portable zip.
# Do not upload the zip to a GitHub Release until Maid approves a tag.
set -euo pipefail
cd "$(dirname "$0")/.."

mkdir -p dist
rm -f dist/Versora-portable.zip

zip -r dist/Versora-portable.zip \
  start.bat start.sh app.py requirements.txt .env.example .gitignore \
  README.md README.zh-Hant.md APPLY.md icon.png \
  src locales projects data scripts \
  -x "*.pyc" "*__pycache__*" "*.env" "*.venv*" "dist/*"

echo "Wrote dist/Versora-portable.zip"
echo "Keep this file local. Do not attach it to a GitHub Release yet."
