#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
export LC_ALL=C.UTF-8
module=target/cxxqt/qml_modules/Snellar
[ -f "$module/qmldir" ] || { echo "qmllint: build first, $module/qmldir is missing"; exit 1; }
qmllint=$(command -v qmllint || true)
for candidate in /usr/lib/qt6/bin/qmllint /opt/qt/*/gcc_64/bin/qmllint; do
    [ -n "$qmllint" ] || { [ -x "$candidate" ] && qmllint=$candidate; } || true
done
[ -n "$qmllint" ] || { echo "qmllint: not found, install qt6-declarative-dev-tools"; exit 1; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/Snellar"
cp -r qml/. "$work/Snellar/"
cp "$module/plugin.qmltypes" .qmllint.ini "$work/Snellar/"
grep -v '^prefer \|^optional plugin \|^classname ' "$module/qmldir" | sed -E 's#^((singleton )?[A-Za-z]+ [0-9.]+ )qml/#\1#' >"$work/Snellar/qmldir"
cd "$work/Snellar"
set +e
"$qmllint" -I "$work" $(find . -name '*.qml' | sed 's#^\./##' | sort) 2>&1 | sed -E -e "s#^(Warning|Info): #\1: qml/#" -e "s#$work/Snellar/#qml/#g"
code=${PIPESTATUS[0]}
exit "$code"
