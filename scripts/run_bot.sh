#!/bin/zsh
set -euo pipefail

cd /Users/octavio/Documents/codes/octaviusp

if [ -f .venv/bin/activate ]; then
  source .venv/bin/activate
fi

exec telecode-bot
