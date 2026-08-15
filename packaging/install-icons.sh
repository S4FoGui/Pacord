#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ICON_ROOT="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
DESKTOP_ROOT="${XDG_DATA_HOME:-$HOME/.local/share}/applications"

install -d "${ICON_ROOT}" "${DESKTOP_ROOT}"
cp -a "${ROOT}/assets/icons/hicolor/." "${ICON_ROOT}/"
install -m 0644 "${ROOT}/packaging/org.pacord.PACORD.desktop" \
  "${DESKTOP_ROOT}/org.pacord.PACORD.desktop"

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "${ICON_ROOT}" >/dev/null 2>&1 || true
fi

printf 'Ícones PACORD instalados em %s\n' "${ICON_ROOT}"
printf 'Arquivo desktop instalado em %s\n' "${DESKTOP_ROOT}/org.pacord.PACORD.desktop"
